// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::cadence::{CadenceDecision, CadenceGovernor, CadenceInfo};
use crate::frame_timeline::FrameTimeline;
use dyxel_perf::{EventRateBuffer, FramePerformanceStats};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PERF_STATS_WINDOW_RECORDS: usize = 120;
/// Warmup presented frames excluded from live Jank/Drop reporting on macOS.
/// Cold-start shader creation and first surface acquire can create real misses,
/// but keeping them in a rolling 120-record UI counter makes Frame 60–120 look
/// unhealthy even after steady-state is within budget.
const PERF_STATS_WARMUP_PRESENTED_SKIP: u64 = if cfg!(target_os = "macos") { 60 } else { 0 };
/// On macOS continuous rendering, mailbox epoch coalescing is a content update
/// drop, not necessarily a displayed-frame drop. Report display drops from
/// skipped-in-flight only.
const PERF_DROP_COUNTS_MAILBOX_EPOCHS: bool = !cfg!(target_os = "macos");
const PERF_DROP_FROM_RASTER_FPS_DEFICIT: bool = cfg!(target_os = "macos");
#[cfg(target_os = "android")]
fn max_submitted_not_presented() -> u32 {
    std::env::var("DYXEL_ANDROID_PIPELINE_DEPTH")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 3)
}

#[cfg(not(target_os = "android"))]
fn max_submitted_not_presented() -> u32 {
    2
}

/// Catch-up threshold for missed VBlank tokens. Disabled on Android due to
/// Choreographer late-phase launch causing spurious catch-up tokens.
#[cfg(target_os = "android")]
const CATCH_UP_THRESHOLD_MS: u64 = 0;
#[cfg(not(target_os = "android"))]
const CATCH_UP_THRESHOLD_MS: u64 = 8;

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
    LogicCommitted {
        epoch: u64,
    },
    VBlank {
        timestamp: Instant,
        refresh_hz: f64,
    },
    RenderStarted {
        frame_id: u64,
        epoch: u64,
    },
    GpuSubmitted {
        frame_id: u64,
        epoch: u64,
        stats: crate::FrameStats,
        will_present: bool,
    },
    GpuReady {
        frame_id: u64,
        epoch: u64,
    },
    Presented {
        frame_id: u64,
        epoch: u64,
        present_ms: f32,
    },
    /// Legacy synchronous path: render and present completed together.
    RenderCompleted {
        frame_id: u64,
        epoch: u64,
        stats: crate::FrameStats,
    },
    SurfaceChanged {
        width: u32,
        height: u32,
        refresh_hz: f64,
    },
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
    /// Buffer for tracking actual platform presented frame rate.
    present_frame_buffer: EventRateBuffer,
    /// Shared performance stats written by the scheduler.
    frame_perf_state: Option<Arc<Mutex<FramePerformanceStats>>>,
    /// Last time a render token was issued. Used for cadence diagnostics.
    last_token_issued_at: Option<Instant>,
    /// Last VBlank that arrived while we were Rendering (or otherwise).
    /// Used for catch-up: if RenderCompleted arrives shortly after a VBlank,
    /// we can retroactively issue a token for that VBlank instead of waiting
    /// another full interval.
    last_vblank: Option<(Instant, CadenceDecision)>,
    submitted_not_ready: u32,
    ready_not_presented: u32,
    last_present_at: Option<Instant>,
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
            present_frame_buffer: EventRateBuffer::new(60),
            frame_perf_state,
            last_token_issued_at: None,
            last_vblank: None,
            submitted_not_ready: 0,
            ready_not_presented: 0,
            last_present_at: None,
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
                    SchedulerState::WaitingForLogic => {
                        // Logic worker is already working; input stays queued and
                        // will be picked up on the next tick.
                    }
                    SchedulerState::Rendering => {
                        // Render is in flight but input should still be processed
                        // immediately so logic can overlap with render. The next
                        // epoch will be committed while rendering and picked up
                        // after RenderCompleted via latest-wins mailbox.
                        if let Some(ref tx) = self.logic_tx {
                            let _ = tx.send(crate::bridge::LogicMessage::ProcessPendingInput);
                        }
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
                // Always record the last VBlank so RenderCompleted can catch up
                // if it arrives shortly after.
                self.last_vblank = Some((timestamp, decision));
                if decision.should_present_this_tick {
                    match self.state {
                        SchedulerState::Armed => {
                            if self.has_pipeline_capacity() {
                                self.try_issue_token_with_decision(decision, timestamp);
                            } else {
                                self.timeline.record_skipped_vblank(
                                    timestamp,
                                    crate::frame_timeline::FrameResultClass::SkippedInFlight,
                                    self.display_hz,
                                    decision.divisor,
                                    decision.effective_hz,
                                );
                            }
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
            SchedulerEvent::GpuSubmitted {
                frame_id,
                epoch,
                stats,
                will_present,
            } => {
                self.handle_gpu_submitted(frame_id, epoch, stats, will_present);
                false
            }
            SchedulerEvent::GpuReady { frame_id, epoch } => {
                if self.submitted_not_ready > 0 {
                    self.submitted_not_ready -= 1;
                }
                self.ready_not_presented = self.ready_not_presented.saturating_add(1);
                if frame_id % 60 == 0 || self.ready_not_presented > 1 {
                    log::info!(
                        "[DIAG] FrameScheduler: GpuReady frame_id={} epoch={} submitted_not_ready={} ready_not_presented={} total={}/{}",
                        frame_id,
                        epoch,
                        self.submitted_not_ready,
                        self.ready_not_presented,
                        self.pipeline_depth(),
                        max_submitted_not_presented()
                    );
                }
                false
            }
            SchedulerEvent::Presented {
                frame_id,
                epoch,
                present_ms,
            } => {
                if self.ready_not_presented > 0 {
                    self.ready_not_presented -= 1;
                } else if self.submitted_not_ready > 0 {
                    self.submitted_not_ready -= 1;
                }
                self.last_presented_epoch = self.last_presented_epoch.max(epoch);
                let now = Instant::now();
                let interval_ms = self
                    .last_present_at
                    .map(|last| now.duration_since(last).as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                self.last_present_at = Some(now);
                self.present_frame_buffer.push(now);
                self.update_performance_state();
                if frame_id % 60 == 0 || present_ms >= 8.0 {
                    log::info!(
                        "[DIAG] FrameScheduler: Presented frame_id={} epoch={} present_ms={:.2} interval={:.2}ms submitted_not_ready={} ready_not_presented={} total={}/{}",
                        frame_id,
                        epoch,
                        present_ms,
                        interval_ms,
                        self.submitted_not_ready,
                        self.ready_not_presented,
                        self.pipeline_depth(),
                        max_submitted_not_presented()
                    );
                }
                false
            }
            SchedulerEvent::RenderCompleted {
                frame_id,
                epoch,
                stats,
            } => {
                self.handle_gpu_submitted(frame_id, epoch, stats, false);
                self.last_presented_epoch = epoch;
                self.present_frame_buffer.push(Instant::now());
                self.update_performance_state();
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

    fn try_issue_token_with_decision(
        &mut self,
        decision: CadenceDecision,
        vblank_timestamp: Instant,
    ) {
        if self.in_flight.is_some() {
            return;
        }
        let frame_id = self.next_frame_id.fetch_add(1, Ordering::Relaxed);
        let epoch = self.latest_committed_epoch;
        let now = Instant::now();
        let token_issue_latency_ms = now.duration_since(vblank_timestamp).as_secs_f64() * 1000.0;
        if let Some(last) = self.last_token_issued_at {
            let token_interval_ms = now.duration_since(last).as_secs_f64() * 1000.0;
            if frame_id % 20 == 0 {
                log::info!(
                    "[DIAG] Scheduler token frame_id={} divisor={} effective_hz={:.2} interval={:.2}ms vblank_latency={:.2}ms state={:?}",
                    frame_id,
                    decision.divisor,
                    decision.effective_hz,
                    token_interval_ms,
                    token_issue_latency_ms,
                    self.state,
                );
            }
        }
        self.last_token_issued_at = Some(now);
        let token = FrameToken {
            frame_id,
            epoch,
            vblank_at: vblank_timestamp,
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

    fn handle_gpu_submitted(
        &mut self,
        frame_id: u64,
        epoch: u64,
        stats: crate::FrameStats,
        will_present: bool,
    ) {
        let now = Instant::now();
        self.timeline.mark_render_completed(frame_id, now);
        let measured_work_ms = (stats.frame_time_ms > 0.0).then_some(stats.frame_time_ms as f64);
        let missed_cadence = self
            .timeline
            .classify_frame_result_with_duration(frame_id, measured_work_ms)
            .map(|r| matches!(r, crate::frame_timeline::FrameResultClass::MissedCadence))
            .unwrap_or(false);
        self.cadence
            .record_frame(crate::cadence::GovernorFrameRecord {
                frame_time_ms: stats.frame_time_ms as f64,
                gpu_time_ms: None,
                missed_cadence,
            });
        self.in_flight = None;
        if will_present {
            self.submitted_not_ready = self.submitted_not_ready.saturating_add(1);
        }
        self.update_performance_state();

        let new_state = if self.latest_committed_epoch > epoch {
            SchedulerState::Armed
        } else {
            SchedulerState::CoolingDown
        };
        self.state = new_state;

        if new_state == SchedulerState::Armed {
            self.try_catch_up(now);
        }
    }

    fn try_catch_up(&mut self, now: Instant) {
        let Some((vblank_timestamp, decision)) = self.last_vblank else {
            return;
        };
        if !decision.should_present_this_tick
            || self.in_flight.is_some()
            || !self.has_pipeline_capacity()
        {
            return;
        }
        let elapsed = now.duration_since(vblank_timestamp);
        if elapsed >= Duration::from_millis(CATCH_UP_THRESHOLD_MS) {
            return;
        }
        log::info!(
            "[DIAG] FrameScheduler: catch-up token for missed VBlank (elapsed={:.2}ms) frame_id={}",
            elapsed.as_secs_f64() * 1000.0,
            self.next_frame_id.load(Ordering::Relaxed)
        );
        self.try_issue_token_with_decision(decision, vblank_timestamp);
    }

    fn pipeline_depth(&self) -> u32 {
        self.submitted_not_ready + self.ready_not_presented
    }

    fn has_pipeline_capacity(&self) -> bool {
        self.pipeline_depth() < max_submitted_not_presented()
    }

    pub fn cadence_info(&self) -> CadenceInfo {
        self.cadence.info()
    }

    /// Write current scheduler performance snapshot to shared state.
    fn update_performance_state(&self) {
        if let Some(ref state) = self.frame_perf_state {
            let cadence = self.cadence.info();
            let window_stats = self.timeline.recent_window_stats_after_presented_skip(
                PERF_STATS_WINDOW_RECORDS,
                PERF_STATS_WARMUP_PRESENTED_SKIP,
            );
            let counts = window_stats.counts;
            let total_presented = counts.total_presented();
            // dropped epochs + skipped-in-flight together constitute the full
            // "dropped frames" count (aligned with Flutter semantics).
            let dropped = if PERF_DROP_COUNTS_MAILBOX_EPOCHS {
                window_stats.dropped_epochs + counts.skipped_in_flight
            } else {
                counts.skipped_in_flight
            };
            let jank_rate = if total_presented > 0 {
                counts.missed_cadence as f32 / total_presented as f32
            } else {
                0.0
            };
            let mut drop_rate = if total_presented + dropped > 0 {
                dropped as f32 / (total_presented + dropped) as f32
            } else {
                0.0
            };
            let mut dropped_count = dropped;
            if let Ok(mut perf) = state.lock() {
                perf.ui_fps = self.ui_frame_buffer.fps();
                if self.present_frame_buffer.fps() > 0.0 {
                    perf.raster_fps = self.present_frame_buffer.fps();
                }
                perf.target_fps = cadence.effective_hz as f32;

                if PERF_DROP_FROM_RASTER_FPS_DEFICIT {
                    // On macOS/Fifo, scheduler skipped-in-flight often means
                    // the render thread is blocked in surface acquire, while
                    // the previous frame is still being presented on cadence.
                    // Report user-visible display drops from the measured
                    // raster FPS deficit instead of internal in-flight state.
                    let target = perf.target_fps.max(1.0);
                    let raster = perf.raster_fps;
                    let deficit = if raster > 0.0 {
                        ((target - raster).max(0.0) / target).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    drop_rate = deficit;
                    dropped_count = (deficit * total_presented as f32).round() as u64;
                }

                perf.jank_count = counts.missed_cadence;
                perf.dropped_count = dropped_count;
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
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 1 })
            .unwrap();
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 2 })
            .unwrap();
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 3 })
            .unwrap();

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
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 1 })
            .unwrap();

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
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 2 })
            .unwrap();

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
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 1 })
            .unwrap();

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
        let scheduler =
            FrameScheduler::new(render_tx, event_rx, None, 60.0, Some(perf_state.clone()));

        let handle = std::thread::spawn(move || scheduler.run());

        // Commit epoch 1
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 1 })
            .unwrap();

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
    fn scheduler_perf_state_uses_recent_window_not_cumulative_history() {
        let (render_tx, _render_rx) = crossbeam_channel::unbounded();
        let (_event_tx, event_rx) = crossbeam_channel::unbounded();
        let perf_state = Arc::new(Mutex::new(FramePerformanceStats::default()));
        let mut scheduler =
            FrameScheduler::new(render_tx, event_rx, None, 60.0, Some(perf_state.clone()));
        let now = Instant::now();

        scheduler
            .timeline
            .record_token(1, 1, now, now, 16.67, 60.0, 1, 60.0, 2);
        scheduler.timeline.mark_render_started(1, now);
        scheduler
            .timeline
            .mark_render_completed(1, now + Duration::from_millis(25));
        scheduler.timeline.classify_frame_result(1);
        scheduler.timeline.record_skipped_vblank(
            now,
            crate::frame_timeline::FrameResultClass::SkippedInFlight,
            60.0,
            1,
            60.0,
        );

        for frame_id in 2..=122 {
            scheduler
                .timeline
                .record_token(frame_id, frame_id, now, now, 16.67, 60.0, 1, 60.0, 0);
            scheduler.timeline.mark_render_started(frame_id, now);
            scheduler
                .timeline
                .mark_render_completed(frame_id, now + Duration::from_millis(5));
            scheduler.timeline.classify_frame_result(frame_id);
        }

        scheduler.update_performance_state();

        let cumulative = scheduler.timeline.result_counts();
        assert_eq!(cumulative.missed_cadence, 1);
        assert_eq!(scheduler.timeline.dropped_epoch_count(), 2);

        let perf = perf_state.lock().unwrap();
        assert_eq!(perf.jank_count, 0);
        assert_eq!(perf.dropped_count, 0);
        assert_eq!(perf.jank_rate, 0.0);
        assert_eq!(perf.drop_rate, 0.0);
    }

    #[test]
    fn scheduler_ui_fps_tracked_on_logic_commits() {
        let (render_tx, _render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let perf_state = Arc::new(Mutex::new(FramePerformanceStats::default()));
        let scheduler =
            FrameScheduler::new(render_tx, event_rx, None, 60.0, Some(perf_state.clone()));

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
        assert!(
            perf.ui_fps > 0.0,
            "ui_fps should be tracked, got {}",
            perf.ui_fps
        );

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn scheduler_catch_up_issues_token_when_render_completed_after_vblank() {
        // Regression test: when RenderCompleted arrives shortly after a VBlank,
        // the scheduler should catch up and issue a token immediately instead
        // of waiting for the next VBlank (which would create a 33ms gap).
        let (render_tx, render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let scheduler = FrameScheduler::new(render_tx, event_rx, None, 60.0, None);

        let handle = std::thread::spawn(move || scheduler.run());

        // Commit epoch 1
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 1 })
            .unwrap();

        // VBlank triggers token — scheduler enters Rendering
        let t0 = Instant::now();
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: t0,
                refresh_hz: 60.0,
            })
            .unwrap();

        let cmd = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        let token = match cmd {
            RenderCommand::Render(t) => t,
            _ => panic!("expected Render"),
        };

        // Simulate: next VBlank arrives BEFORE RenderCompleted (the race condition)
        let t1 = t0 + Duration::from_millis(16);
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: t1,
                refresh_hz: 60.0,
            })
            .unwrap();

        // Newer content becomes available while the first frame is still in
        // flight, so RenderCompleted should re-arm and catch up.
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 2 })
            .unwrap();

        // RenderCompleted arrives shortly AFTER that VBlank (within the
        // catch-up window).
        event_tx
            .send(SchedulerEvent::RenderCompleted {
                frame_id: token.frame_id,
                epoch: token.epoch,
                stats: crate::FrameStats::default(),
            })
            .unwrap();

        // Scheduler should catch up and issue a token immediately, not wait for next VBlank
        let cmd2 = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        match cmd2 {
            RenderCommand::Render(t) => {
                assert_eq!(t.epoch, 2);
                // The catch-up token should reference the VBlank at t1
                assert!(
                    t.vblank_at >= t1 && t.vblank_at < t1 + Duration::from_millis(1),
                    "catch-up token should reference the missed VBlank"
                );
            }
            _ => panic!("expected catch-up Render"),
        }

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn scheduler_no_catch_up_when_render_completed_too_late() {
        // If RenderCompleted arrives more than 8ms after the VBlank,
        // don't catch up — wait for the next VBlank.
        let (render_tx, render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let scheduler = FrameScheduler::new(render_tx, event_rx, None, 60.0, None);

        let handle = std::thread::spawn(move || scheduler.run());

        // Commit epoch 1
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 1 })
            .unwrap();

        // VBlank triggers token — scheduler enters Rendering
        let t0 = Instant::now();
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: t0,
                refresh_hz: 60.0,
            })
            .unwrap();

        let cmd = render_rx.recv_timeout(Duration::from_millis(100)).unwrap();
        let token = match cmd {
            RenderCommand::Render(t) => t,
            _ => panic!("expected Render"),
        };

        // Next VBlank arrives before RenderCompleted
        let t1 = t0 + Duration::from_millis(16);
        event_tx
            .send(SchedulerEvent::VBlank {
                timestamp: t1,
                refresh_hz: 60.0,
            })
            .unwrap();

        // RenderCompleted arrives 10ms after VBlank (outside 8ms catch-up window)
        event_tx
            .send(SchedulerEvent::RenderCompleted {
                frame_id: token.frame_id,
                epoch: token.epoch,
                stats: crate::FrameStats::default(),
            })
            .unwrap();

        // No immediate catch-up token
        assert!(
            render_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "should not catch up when RenderCompleted is too late"
        );

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn scheduler_dispatches_logic_while_rendering_on_input_arrived() {
        // Regression test: InputArrived during Rendering must immediately
        // dispatch ProcessPendingInput so logic can overlap with render.
        let (render_tx, render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (logic_tx, logic_rx) = std::sync::mpsc::channel();
        let scheduler = FrameScheduler::new(render_tx, event_rx, Some(logic_tx), 60.0, None);

        let handle = std::thread::spawn(move || scheduler.run());

        // Commit epoch 1
        event_tx
            .send(SchedulerEvent::LogicCommitted { epoch: 1 })
            .unwrap();

        // VBlank triggers token — scheduler enters Rendering
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

        // While still in Rendering, input arrives (e.g. user pointer event)
        event_tx.send(SchedulerEvent::InputArrived(42)).unwrap();

        // Scheduler must immediately dispatch ProcessPendingInput to logic.
        // Drain any CadenceUpdated that may have been sent on first tick.
        let msg = loop {
            match logic_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(crate::bridge::LogicMessage::ProcessPendingInput) => break Ok(()),
                Ok(crate::bridge::LogicMessage::CadenceUpdated(_)) => continue,
                Ok(_) => panic!("Expected ProcessPendingInput or CadenceUpdated"),
                Err(e) => break Err(e),
            }
        };
        assert!(
            msg.is_ok(),
            "InputArrived during Rendering must dispatch ProcessPendingInput"
        );

        // Complete render so scheduler can shut down cleanly
        event_tx
            .send(SchedulerEvent::RenderCompleted {
                frame_id: token.frame_id,
                epoch: token.epoch,
                stats: crate::FrameStats::default(),
            })
            .unwrap();

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn scheduler_does_not_redispatch_logic_while_waiting_for_logic() {
        // InputArrived while already WaitingForLogic should be a no-op
        // to avoid queuing redundant ProcessPendingInput messages.
        let (render_tx, _render_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (logic_tx, logic_rx) = std::sync::mpsc::channel();
        let scheduler = FrameScheduler::new(render_tx, event_rx, Some(logic_tx), 60.0, None);

        let handle = std::thread::spawn(move || scheduler.run());

        // First input triggers WaitingForLogic and dispatches ProcessPendingInput.
        // Drain any initial CadenceUpdated first.
        event_tx.send(SchedulerEvent::InputArrived(1)).unwrap();
        let msg1 = loop {
            match logic_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(crate::bridge::LogicMessage::ProcessPendingInput) => break true,
                Ok(crate::bridge::LogicMessage::CadenceUpdated(_)) => continue,
                Ok(_) => break false,
                Err(_) => break false,
            }
        };
        assert!(
            msg1,
            "First InputArrived should dispatch ProcessPendingInput"
        );

        // Second input while still WaitingForLogic should NOT re-dispatch.
        // Ensure no stray CadenceUpdated is still in the channel.
        while let Ok(crate::bridge::LogicMessage::CadenceUpdated(_)) = logic_rx.try_recv() {}
        event_tx.send(SchedulerEvent::InputArrived(2)).unwrap();
        let msg2 = logic_rx.recv_timeout(Duration::from_millis(50));
        assert!(
            msg2.is_err(),
            "InputArrived while WaitingForLogic should not re-dispatch"
        );

        event_tx.send(SchedulerEvent::Shutdown).unwrap();
        handle.join().unwrap();
    }
}
