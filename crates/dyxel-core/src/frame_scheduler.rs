// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::cadence::{CadenceDecision, CadenceGovernor, CadenceInfo};
use crate::frame_timeline::FrameTimeline;
use dyxel_perf::{EventRateBuffer, FramePerformanceStats};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerState {
    Idle,
    WaitingForLogic,
    Armed,
    Rendering,
    CoolingDown,
}

#[derive(Debug, Clone, Copy)]
pub struct FrameToken {
    pub frame_id: u64,
    pub epoch: u64,
    pub vblank_at: Instant,
    pub target_frame_duration: Duration,
}

pub type InputBatchId = u64;

#[derive(Debug)]
pub enum SchedulerEvent {
    InputArrived(InputBatchId),
    LogicCommitted { epoch: u64 },
    VBlank { timestamp: Instant, refresh_hz: f64 },
    RenderStarted { frame_id: u64, epoch: u64 },
    RenderCompleted { frame_id: u64, epoch: u64, stats: crate::FrameStats },
    SurfaceChanged { width: u32, height: u32, refresh_hz: f64 },
    Shutdown,
}

#[derive(Debug)]
pub enum LogicCommand {
    ProcessInput,
    Shutdown,
}

#[derive(Debug)]
pub enum RenderCommand {
    Render(FrameToken),
    Shutdown,
}

/// Tracks an in-flight frame from token issue to render completion.
#[derive(Debug)]
#[allow(dead_code)]
struct InFlightFrame {
    frame_id: u64,
    epoch: u64,
    token_issued_at: Instant,
}

/// FrameScheduler is the single frame owner. It decides when frames are issued,
/// tracks in-flight work, and maintains cadence via CadenceGovernor.
pub struct FrameScheduler {
    state: SchedulerState,
    cadence: CadenceGovernor,
    timeline: FrameTimeline,
    render_cmd_tx: crossbeam_channel::Sender<RenderCommand>,
    event_rx: crossbeam_channel::Receiver<SchedulerEvent>,
    /// Channel to dispatch work to the logic worker (e.g. ProcessPendingInput).
    logic_tx: Option<std::sync::mpsc::Sender<crate::bridge::LogicMessage>>,
    in_flight: Option<InFlightFrame>,
    last_presented_epoch: u64,
    latest_committed_epoch: u64,
    next_frame_id: AtomicU64,
    display_hz: f64,
    surface_width: u32,
    surface_height: u32,
    /// Snapshot of the last cadence (display_hz, divisor) pushed to the logic
    /// worker. Used to detect *any* meaningful cadence change (display_hz or
    /// divisor) without being tripped up by the fact that handle_event updates
    /// self.display_hz and self.cadence in lockstep.
    last_sent_cadence: Option<(f64, u32)>,
    /// Buffer for tracking UI (logic commit) frame rate.
    ui_frame_buffer: EventRateBuffer,
    /// Shared performance stats written by the scheduler.
    frame_perf_state: Option<Arc<Mutex<FramePerformanceStats>>>,
}

impl FrameScheduler {
    pub fn new(
        render_cmd_tx: crossbeam_channel::Sender<RenderCommand>,
        event_rx: crossbeam_channel::Receiver<SchedulerEvent>,
        logic_tx: Option<std::sync::mpsc::Sender<crate::bridge::LogicMessage>>,
        display_hz: f64,
        frame_perf_state: Option<Arc<Mutex<FramePerformanceStats>>>,
    ) -> Self {
        Self {
            state: SchedulerState::Idle,
            cadence: CadenceGovernor::new(display_hz),
            timeline: FrameTimeline::new(),
            render_cmd_tx,
            event_rx,
            logic_tx,
            in_flight: None,
            last_presented_epoch: 0,
            latest_committed_epoch: 0,
            next_frame_id: AtomicU64::new(1),
            display_hz,
            surface_width: 0,
            surface_height: 0,
            last_sent_cadence: None,
            ui_frame_buffer: EventRateBuffer::new(60),
            frame_perf_state,
        }
    }

    /// Run the scheduler event loop. Blocks until Shutdown is received.
    pub fn run(mut self) {
        log::info!("FrameScheduler: started");
        loop {
            match self.event_rx.recv() {
                Ok(event) => {
                    if self.handle_event(event) {
                        break;
                    }
                    self.check_and_notify_cadence_change();
                }
                Err(e) => {
                    log::warn!("FrameScheduler: event channel error: {}", e);
                    break;
                }
            }
        }
        log::info!("FrameScheduler: shutting down");
    }

    /// If cadence (divisor or display_hz) has changed, send CadenceUpdated
    /// to the logic worker so animation logic uses the real effective_hz.
    fn check_and_notify_cadence_change(&mut self) {
        let info = self.cadence.info();
        let current = (info.display_hz, info.divisor);
        let changed = match self.last_sent_cadence {
            Some(last) => last != current,
            None => true,
        };
        if changed {
            self.last_sent_cadence = Some(current);
            if let Some(ref tx) = self.logic_tx {
                let _ = tx.send(crate::bridge::LogicMessage::CadenceUpdated(info));
            }
        }
    }

    /// Handle a single scheduler event. Returns true if the loop should exit.
    fn handle_event(&mut self, event: SchedulerEvent) -> bool {
        match event {
            SchedulerEvent::InputArrived(_batch_id) => {
                // Input is noted but does not directly trigger a frame.
                // Dispatch ProcessPendingInput to the logic worker whenever it
                // is not already working (Idle, Armed, or CoolingDown).
                // In Rendering state the input will be picked up after
                // RenderCompleted when the scheduler transitions back.
                match self.state {
                    SchedulerState::Idle => {
                        self.state = SchedulerState::WaitingForLogic;
                        if let Some(ref tx) = self.logic_tx {
                            let _ = tx.send(crate::bridge::LogicMessage::ProcessPendingInput);
                        }
                    }
                    SchedulerState::Armed => {
                        // New input arrived while we have an armed package.
                        // Re-dispatch logic to process the new input and
                        // commit a fresher epoch.
                        if let Some(ref tx) = self.logic_tx {
                            let _ = tx.send(crate::bridge::LogicMessage::ProcessPendingInput);
                        }
                    }
                    SchedulerState::CoolingDown => {
                        self.state = SchedulerState::WaitingForLogic;
                        if let Some(ref tx) = self.logic_tx {
                            let _ = tx.send(crate::bridge::LogicMessage::ProcessPendingInput);
                        }
                    }
                    SchedulerState::WaitingForLogic | SchedulerState::Rendering => {
                        // Logic worker is already working or render is in flight;
                        // input stays queued and will be processed when ready.
                    }
                }
                false
            }
            SchedulerEvent::LogicCommitted { epoch } => {
                self.latest_committed_epoch = epoch;
                self.ui_frame_buffer.push(Instant::now());
                self.update_performance_state();
                match self.state {
                    SchedulerState::Idle | SchedulerState::WaitingForLogic => {
                        self.state = SchedulerState::Armed;
                        // Do NOT issue immediately. Frame tokens are only
                        // issued on VBlank ticks (refresh-locked contract).
                    }
                    SchedulerState::Armed => {
                        // Higher epoch supersedes the old armed epoch.
                    }
                    SchedulerState::Rendering => {
                        // New content arrived while a frame is in flight.
                        // It will be picked up after RenderCompleted.
                    }
                    SchedulerState::CoolingDown => {
                        self.state = SchedulerState::Armed;
                    }
                }
                false
            }
            SchedulerEvent::VBlank {
                timestamp,
                refresh_hz,
            } => {
                if (refresh_hz - self.display_hz).abs() > 1.0 {
                    log::info!(
                        "FrameScheduler: display refresh changed {} -> {}",
                        self.display_hz,
                        refresh_hz
                    );
                    self.display_hz = refresh_hz;
                    self.cadence = CadenceGovernor::new(refresh_hz);
                }
                let decision = self.cadence.on_vblank(timestamp);
                if decision.should_present_this_tick {
                    match self.state {
                        SchedulerState::Armed => {
                            self.try_issue_token_with_decision(decision);
                        }
                        SchedulerState::Rendering => {
                            // A frame is already in flight; this VBlank cannot
                            // start a new one.
                            self.timeline.record_skipped_vblank(
                                timestamp,
                                crate::frame_timeline::FrameResultClass::SkippedInFlight,
                                self.display_hz,
                                decision.divisor,
                                decision.effective_hz,
                            );
                            self.cadence.record_skipped_in_flight();
                        }
                        _ => {
                            // Idle, WaitingForLogic, or CoolingDown — no new
                            // content ready to display.
                            self.timeline.record_skipped_vblank(
                                timestamp,
                                crate::frame_timeline::FrameResultClass::SkippedIdle,
                                self.display_hz,
                                decision.divisor,
                                decision.effective_hz,
                            );
                        }
                    }
                } else {
                    // VBlank suppressed by divisor.
                    self.timeline.record_skipped_vblank(
                        timestamp,
                        crate::frame_timeline::FrameResultClass::SkippedDivisor,
                        self.display_hz,
                        decision.divisor,
                        decision.effective_hz,
                    );
                }
                false
            }
            SchedulerEvent::RenderStarted { frame_id, epoch } => {
                self.timeline.mark_render_started(frame_id, Instant::now());
                log::trace!(
                    "FrameScheduler: RenderStarted frame_id={} epoch={}",
                    frame_id,
                    epoch
                );
                false
            }
            SchedulerEvent::RenderCompleted {
                frame_id,
                epoch,
                stats,
            } => {
                let now = Instant::now();
                self.timeline.mark_render_completed(frame_id, now);
                let missed_cadence = self
                    .timeline
                    .classify_frame_result(frame_id)
                    .map(|r| matches!(r, crate::frame_timeline::FrameResultClass::MissedCadence))
                    .unwrap_or(false);
                self.cadence.record_frame(crate::cadence::GovernorFrameRecord {
                    frame_time_ms: stats.frame_time_ms as f64,
                    gpu_time_ms: None, // TODO: populate when GPU timing is available
                    missed_cadence,
                });
                self.last_presented_epoch = epoch;
                self.in_flight = None;
                self.update_performance_state();

                // If a newer epoch was committed while rendering, arm again.
                // The next VBlank will issue the token (refresh-locked).
                if self.latest_committed_epoch > epoch {
                    self.state = SchedulerState::Armed;
                } else {
                    self.state = SchedulerState::CoolingDown;
                }
                false
            }
            SchedulerEvent::SurfaceChanged {
                width,
                height,
                refresh_hz,
            } => {
                self.surface_width = width;
                self.surface_height = height;
                if (refresh_hz - self.display_hz).abs() > 1.0 {
                    log::info!(
                        "FrameScheduler: surface changed, new refresh {}Hz {}x{}",
                        refresh_hz,
                        width,
                        height
                    );
                    self.display_hz = refresh_hz;
                    self.cadence = CadenceGovernor::new(refresh_hz);
                }
                false
            }
            SchedulerEvent::Shutdown => true,
        }
    }

    fn try_issue_token_with_decision(&mut self, decision: CadenceDecision) {
        if self.in_flight.is_some() {
            return;
        }
        let frame_id = self.next_frame_id.fetch_add(1, Ordering::Relaxed);
        let epoch = self.latest_committed_epoch;
        let now = Instant::now();
        let token = FrameToken {
            frame_id,
            epoch,
            vblank_at: now,
            target_frame_duration: decision.target_frame_duration,
        };

        let target_ms = decision.target_frame_duration.as_secs_f64() * 1000.0;
        let dropped_epochs = self
            .latest_committed_epoch
            .saturating_sub(self.last_presented_epoch + 1);
        self.timeline.record_token(
            frame_id,
            epoch,
            now,
            now,
            target_ms,
            self.display_hz,
            decision.divisor,
            decision.effective_hz,
            dropped_epochs,
        );
        match self.render_cmd_tx.send(RenderCommand::Render(token)) {
            Ok(_) => {
                self.in_flight = Some(InFlightFrame {
                    frame_id,
                    epoch,
                    token_issued_at: now,
                });
                self.state = SchedulerState::Rendering;
                log::debug!(
                    "FrameScheduler: issued token frame_id={} epoch={} divisor={}",
                    frame_id,
                    epoch,
                    decision.divisor
                );
            }
            Err(e) => {
                log::warn!("FrameScheduler: failed to send render command: {}", e);
            }
        }
    }

    pub fn cadence_info(&self) -> CadenceInfo {
        self.cadence.info()
    }

    /// Write current scheduler performance snapshot to shared state.
    fn update_performance_state(&self,
    ) {
        if let Some(ref state) = self.frame_perf_state {
            let cadence = self.cadence.info();
            let counts = self.timeline.result_counts();
            let total_presented = counts.total_presented();
            // dropped epochs + skipped-in-flight together constitute the full
            // "dropped frames" count (aligned with Flutter semantics).
            let dropped = self.timeline.dropped_epoch_count() + counts.skipped_in_flight;
            let jank_rate = if total_presented > 0 {
                counts.missed_cadence as f32 / total_presented as f32
            } else {
                0.0
            };
            let drop_rate = if total_presented + dropped > 0 {
                dropped as f32 / (total_presented + dropped) as f32
            } else {
                0.0
            };
            if let Ok(mut perf) = state.lock() {
                perf.ui_fps = self.ui_frame_buffer.fps();
                perf.target_fps = cadence.effective_hz as f32;
                perf.jank_count = counts.missed_cadence;
                perf.dropped_count = dropped;
                perf.jank_rate = jank_rate;
                perf.drop_rate = drop_rate;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_coalesces_multiple_logic_commits_while_render_in_flight() {
        let (render_tx, render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let scheduler = FrameScheduler::new(render_tx, event_rx, None, 60.0, None);

        let handle = std::thread::spawn(move || scheduler.run());

        // Commit epochs 1, 2, 3 rapidly — scheduler arms but does NOT issue yet.
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 1 }).unwrap();
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 2 }).unwrap();
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 3 }).unwrap();

        // VBlank triggers the first token (refresh-locked)
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: Instant::now(),
                refresh_hz: 60.0,
            })
            .unwrap();

        // First token is epoch 3 (coalesced from 1,2,3)
        let cmd1 = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        let token1 = match cmd1 {
            RenderCommand::Render(t) => t,
            _ => panic!("expected Render command"),
        };
        assert_eq!(token1.epoch, 3);

        // Complete frame 1 — scheduler arms for next VBlank
        event_tx
            .send(SchedulerEvent::RenderCompleted {
                frame_id: token1.frame_id,
                epoch: token1.epoch,
                stats: crate::FrameStats::default(),
            })
            .unwrap();

        // No second command until next VBlank
        assert!(
            render_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "should not receive second command without VBlank"
        );

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn scheduler_never_issues_second_token_while_render_in_flight() {
        let (render_tx, render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let scheduler = FrameScheduler::new(render_tx, event_rx, None, 60.0, None);

        let handle = std::thread::spawn(move || scheduler.run());

        // Commit epoch 1
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 1 }).unwrap();

        // VBlank triggers token for epoch 1
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: Instant::now(),
                refresh_hz: 60.0,
            })
            .unwrap();

        let cmd1 = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        let token1 = match cmd1 {
            RenderCommand::Render(t) => t,
            _ => panic!("expected Render"),
        };

        // Commit epoch 2 while frame 1 is still in flight
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 2 }).unwrap();

        // Should NOT receive a second token immediately (no VBlank yet)
        assert!(
            render_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "should not issue second token while in flight"
        );

        // Complete frame 1 — scheduler arms for next VBlank
        event_tx
            .send(SchedulerEvent::RenderCompleted {
                frame_id: token1.frame_id,
                epoch: token1.epoch,
                stats: crate::FrameStats::default(),
            })
            .unwrap();

        // Next VBlank triggers token for epoch 2
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: Instant::now(),
                refresh_hz: 60.0,
            })
            .unwrap();

        let cmd2 = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        match cmd2 {
            RenderCommand::Render(t) => {
                assert_eq!(t.epoch, 2);
            }
            _ => panic!("expected Render"),
        }

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn scheduler_vblank_driven_does_not_issue_on_logic_committed() {
        let (render_tx, render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let scheduler = FrameScheduler::new(render_tx, event_rx, None, 60.0, None);

        let handle = std::thread::spawn(move || scheduler.run());

        // Prime the scheduler so it knows VBlank is available.
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: Instant::now(),
                refresh_hz: 60.0,
            })
            .unwrap();

        // Commit epoch 1 — now scheduler is VBlank-driven, so it only arms.
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 1 }).unwrap();

        // No token yet without a VBlank.
        assert!(
            render_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "should not issue token without VBlank in VBlank-driven mode"
        );

        // VBlank triggers the token.
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: Instant::now(),
                refresh_hz: 60.0,
            })
            .unwrap();

        let cmd1 = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        match cmd1 {
            RenderCommand::Render(t) => {
                assert_eq!(t.epoch, 1);
            }
            _ => panic!("expected Render"),
        }

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn scheduler_writes_performance_state_on_render_completed() {
        let (render_tx, render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let perf_state = Arc::new(Mutex::new(FramePerformanceStats::default()));
        let scheduler = FrameScheduler::new(
            render_tx,
            event_rx,
            None,
            60.0,
            Some(perf_state.clone()),
        );

        let handle = std::thread::spawn(move || scheduler.run());

        // Commit epoch 1
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 1 }).unwrap();

        // VBlank triggers token
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: Instant::now(),
                refresh_hz: 60.0,
            })
            .unwrap();

        let cmd = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        let token = match cmd {
            RenderCommand::Render(t) => t,
            _ => panic!("expected Render"),
        };

        // Start and complete render
        event_tx
            .send(SchedulerEvent::RenderStarted {
                frame_id: token.frame_id,
                epoch: token.epoch,
            })
            .unwrap();
        event_tx
            .send(SchedulerEvent::RenderCompleted {
                frame_id: token.frame_id,
                epoch: token.epoch,
                stats: crate::FrameStats::default(),
            })
            .unwrap();

        // Give scheduler a moment to process
        std::thread::sleep(Duration::from_millis(20));

        // Verify shared state was updated
        let perf = perf_state.lock().unwrap();
        assert_eq!(perf.target_fps, 60.0);
        // ui_fps may be 0.0 because only 1 commit doesn't give us 2 events for fps calc
        assert_eq!(perf.jank_count, 0);
        assert_eq!(perf.dropped_count, 0);

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn scheduler_ui_fps_tracked_on_logic_commits() {
        let (render_tx, _render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let perf_state = Arc::new(Mutex::new(FramePerformanceStats::default()));
        let scheduler = FrameScheduler::new(
            render_tx,
            event_rx,
            None,
            60.0,
            Some(perf_state.clone()),
        );

        let handle = std::thread::spawn(move || scheduler.run());

        // Commit several epochs rapidly
        for epoch in 1..=5 {
            event_tx
                .send(SchedulerEvent::LogicCommitted { epoch })
                .unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }

        std::thread::sleep(Duration::from_millis(20));

        let perf = perf_state.lock().unwrap();
        // With 5 commits over ~50ms, fps should be ~4/0.05 = 80 (very rough)
        assert!(perf.ui_fps > 0.0, "ui_fps should be tracked, got {}", perf.ui_fps);

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }
}
