// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

pub mod platform;
pub mod state;
pub mod runtime;
pub mod transaction;
pub mod renderer;
pub mod engine;
pub mod input;
pub mod bridge;
// Perf module now in dyxel-perf crate

pub use platform::{SurfaceId, SafeWindowHandle, SurfaceState};
pub use state::{SharedState, ViewNode};
pub use engine::{LogicState, RenderState, setup_engine};
pub use bridge::DyxelHost;
pub use dyxel_perf::{PerformanceMonitor, SharedPerfMonitor, PerfConfig, FrameStats};

// Re-exports for other crates (like host-web)
pub use state::{Role, ViewType};
pub use input::hit_test_recursive;
pub use runtime::{
    process_commands, sync_layout_to_wasm, process_command_stream,
    is_render_needed, get_dirty_tracker, clear_dirty_tracker
};

#[cfg(not(target_arch = "wasm32"))]
uniffi::setup_scaffolding!("dyxel_core");
