// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#[cfg(target_os = "android")]
pub mod android_vblank;
pub mod bridge;
pub mod engine;
pub mod frame_scheduler;
pub mod handler_registry;
pub mod input;
pub mod input_proxy;
pub mod pacer;
pub mod platform;
pub mod renderer;
pub mod render_mailbox;
pub mod runtime;
pub mod spatial_index;
pub mod state;
pub mod transaction;
// Perf module now in dyxel-perf crate

pub use bridge::DyxelHost;
pub use dyxel_perf::{FrameStats, PerfConfig, PerformanceMonitor, SharedPerfMonitor};
pub use engine::{setup_engine, LogicState, RenderState};
pub use platform::{SafeWindowHandle, SurfaceId, SurfaceState};
pub use state::{SharedState, ViewNode};

// Re-exports for other crates (like host-web)
pub use input::hit_test_recursive;
pub use runtime::{
    clear_dirty_tracker, is_render_needed, mark_all_nodes_dirty,
    process_command_stream, process_commands, sync_layout_to_wasm,
};
pub use state::{Role, ViewType};

#[cfg(not(target_arch = "wasm32"))]
uniffi::setup_scaffolding!("dyxel_core");
