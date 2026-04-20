// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::cadence::{CadenceDecision, CadenceGovernor, CadenceInfo};
use crate::frame_timeline::{FrameRecord, FrameTimeline};
use std::sync::atomic::{AtomicU64, Ordering};
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
    in_flight: Option<InFlightFrame>,
    last_presented_epoch: u64,
    latest_committed_epoch: u64,
    next_frame_id: AtomicU64,
    display_hz: f64,
    surface_width: u32,
    surface_height: u32,
}

impl FrameScheduler {
    pub fn new(
        render_cmd_tx: crossbeam_channel::Sender<RenderCommand>,
        event_rx: crossbeam_channel::Receiver<SchedulerEvent>,
        display_hz: f64,
    ) -> Self {
        Self {
            state: SchedulerState::Idle,
            cadence: CadenceGovernor::new(display_hz),
            timeline: FrameTimeline::new(),
            render_cmd_tx,
            event_rx,
            in_flight: None,
            last_presented_epoch: 0,
            latest_committed_epoch: 0,
            next_frame_id: AtomicU64::new(1),
            display_hz,
            surface_width: 0,
            surface_height: 0,
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
                }
                Err(e) => {
                    log::warn!("FrameScheduler: event channel error: {}", e);
                    break;
                }
            }
        }
        log::info!("FrameScheduler: shutting down");
    }

    /// Handle a single scheduler event. Returns true if the loop should exit.
    fn handle_event(&mut self, event: SchedulerEvent) -> bool {
        match event {
            SchedulerEvent::InputArrived(_batch_id) => {
                // Input is noted but does not directly trigger a frame.
                // If we're Idle, transition to WaitingForLogic (the logic worker
                // will eventually commit a package).
                if self.state == SchedulerState::Idle {
                    self.state = SchedulerState::WaitingForLogic;
                }
                false
            }
            SchedulerEvent::LogicCommitted { epoch } => {
                self.latest_committed_epoch = epoch;
                match self.state {
                    SchedulerState::Idle | SchedulerState::WaitingForLogic => {
                        self.state = SchedulerState::Armed;
                        // If no frame is in flight, attempt to issue immediately.
                        // In a real VBlank-driven system we'd wait for the next tick,
                        // but for startup / low-latency paths we issue right away
                        // when the cadence allows.
                        self.try_issue_token();
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
                        self.try_issue_token();
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
                if decision.should_present_this_tick && self.state == SchedulerState::Armed {
                    self.try_issue_token_with_decision(decision);
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
                self.record_frame_result(frame_id, epoch, now);
                self.cadence.record_frame_duration(stats.frame_time_ms as f64);
                self.last_presented_epoch = epoch;
                self.in_flight = None;

                // If a newer epoch was committed while rendering, arm again.
                if self.latest_committed_epoch > epoch {
                    self.state = SchedulerState::Armed;
                    self.try_issue_token();
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

    /// Attempt to issue a frame token using the current cadence decision.
    fn try_issue_token(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        let decision = self.cadence.current_decision();
        self.try_issue_token_with_decision(decision);
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

        self.timeline.record_token(frame_id, epoch, now, now);
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

    fn record_frame_result(&mut self, frame_id: u64, epoch: u64, completed_at: Instant) {
        if let Some(rec) = self.timeline.recent_mut().iter_mut().find(|r| r.frame_id == frame_id) {
            rec.frame_result = Some(crate::frame_timeline::FrameResultClass::OnTime);
            rec.presented_at = Some(completed_at);
            rec.epoch = epoch;
        }
    }

    pub fn cadence_info(&self) -> CadenceInfo {
        self.cadence.info()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_coalesces_multiple_logic_commits_while_render_in_flight() {
        let (render_tx, render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let scheduler = FrameScheduler::new(render_tx, event_rx, 60.0);

        // Spawn scheduler in background
        let handle = std::thread::spawn(move || scheduler.run());

        // Commit epochs 1, 2, 3 rapidly
        // Epoch 1 issues immediately (eager low-latency path)
        // Epochs 2 and 3 are absorbed while the first frame is in flight
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 1 }).unwrap();
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 2 }).unwrap();
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 3 }).unwrap();

        // First token is epoch 1 (eager issue on first commit)
        let cmd1 = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        let token1 = match cmd1 {
            RenderCommand::Render(t) => t,
            _ => panic!("expected Render command"),
        };
        assert_eq!(token1.epoch, 1);

        // Complete frame 1 — scheduler should issue the latest coalesced epoch
        event_tx
            .send(SchedulerEvent::RenderCompleted {
                frame_id: token1.frame_id,
                epoch: token1.epoch,
                stats: crate::FrameStats::default(),
            })
            .unwrap();

        // Next token should be epoch 3 (coalesced from 2 and 3)
        let cmd2 = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        match cmd2 {
            RenderCommand::Render(token) => {
                assert_eq!(token.epoch, 3);
            }
            _ => panic!("expected Render command"),
        }

        // No third command should arrive
        assert!(
            render_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "should not receive third command"
        );

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn scheduler_never_issues_second_token_while_render_in_flight() {
        let (render_tx, render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let scheduler = FrameScheduler::new(render_tx, event_rx, 60.0);

        let handle = std::thread::spawn(move || scheduler.run());

        // Commit epoch 1 -> token issued
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 1 }).unwrap();
        let cmd1 = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        let token1 = match cmd1 {
            RenderCommand::Render(t) => t,
            _ => panic!("expected Render"),
        };

        // Commit epoch 2 while frame 1 is still in flight
        event_tx.send(SchedulerEvent::LogicCommitted { epoch: 2 }).unwrap();

        // Should NOT receive a second token immediately
        assert!(
            render_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "should not issue second token while in flight"
        );

        // Complete frame 1
        event_tx
            .send(SchedulerEvent::RenderCompleted {
                frame_id: token1.frame_id,
                epoch: token1.epoch,
                stats: crate::FrameStats::default(),
            })
            .unwrap();

        // NOW should receive token for epoch 2
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
}
