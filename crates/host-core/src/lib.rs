pub mod platform;
pub mod state;
pub mod runtime;
pub mod renderer;
pub mod engine;
pub mod input;
pub mod bridge;

pub use platform::{SurfaceId, SafeWindowHandle, SurfaceState};
pub use state::{SharedState, ViewNode};
pub use engine::{EngineState, setup_engine};
pub use bridge::VelloHost;

// Re-exports for other crates (like host-web)
pub use state::{Role, ViewType};
pub use input::hit_test_recursive;
pub use runtime::{process_commands, sync_layout_to_wasm, process_command_stream};

#[cfg(not(target_arch = "wasm32"))]
uniffi::setup_scaffolding!();
