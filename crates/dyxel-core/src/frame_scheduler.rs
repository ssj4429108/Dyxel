// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

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

#[derive(Debug)]
pub enum SchedulerEvent {
    LogicCommitted { epoch: u64 },
    RenderCompleted { frame_id: u64, epoch: u64 },
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
