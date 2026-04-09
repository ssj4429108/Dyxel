// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

pub mod bridge;
pub mod engine;
pub mod handler_registry;
pub mod input;
pub mod input_proxy;
pub mod keyboard;
pub mod platform;
pub mod renderer;
pub mod runtime;
pub mod spatial_index;
pub mod state;
pub mod text_input;
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
    clear_dirty_tracker, get_dirty_tracker, is_render_needed, mark_all_nodes_dirty,
    process_command_stream, process_commands, sync_layout_to_wasm,
};
pub use state::{Role, ViewType};

use std::cell::RefCell;
use crate::input_proxy::{InputProxy, InputProxyConfig};
use dyxel_render_api::{SharedMutex, SharedPtr};

/// Bridge handle for accessing input proxy and shared state
/// Used by platform layers to send input events
pub struct BridgeHandle {
    shared_state: SharedPtr<SharedMutex<SharedState>>,
    input_proxy: RefCell<InputProxy>,
}

impl BridgeHandle {
    pub fn new(shared_state: SharedPtr<SharedMutex<SharedState>>) -> Self {
        let input_proxy = InputProxy::new(InputProxyConfig::default());
        Self {
            shared_state,
            input_proxy: RefCell::new(input_proxy),
        }
    }

    /// Get the shared state pointer for accessing shared buffer
    fn with_shared_buffer<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut dyxel_shared::SharedBuffer) -> R,
    {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let state = self.shared_state.lock().ok()?;
            let buffer_ptr = state.get_shared_buffer_ptr()?;
            if buffer_ptr.is_null() {
                return None;
            }
            // SAFETY: We assume the pointer is valid when non-null
            let buffer = unsafe { &mut *buffer_ptr };
            Some(f(buffer))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let state = self.shared_state.try_borrow_mut().ok()?;
            let buffer_ptr = state.get_shared_buffer_ptr()?;
            if buffer_ptr.is_null() {
                return None;
            }
            // SAFETY: We assume the pointer is valid when non-null
            let buffer = unsafe { &mut *buffer_ptr };
            Some(f(buffer))
        }
    }
}

thread_local! {
    /// Thread-local bridge handle for input event handling
    static BRIDGE: RefCell<Option<BridgeHandle>> = RefCell::new(None);
}

/// Initialize the bridge handle (called during engine setup)
pub fn init_bridge(shared_state: SharedPtr<SharedMutex<SharedState>>) {
    BRIDGE.with(|b| {
        *b.borrow_mut() = Some(BridgeHandle::new(shared_state));
    });
}

/// 处理键盘按下事件（由平台层调用）
pub fn handle_key_down(key_code: u32, modifiers: u8) {
    BRIDGE.with(|b| {
        if let Some(ref bridge) = *b.borrow() {
            let mut proxy = bridge.input_proxy.borrow_mut();
            bridge.with_shared_buffer(|buffer| {
                proxy.handle_key_down(key_code, modifiers, buffer);
            });
        }
    });
}

/// 处理键盘释放事件（由平台层调用）
pub fn handle_key_up(key_code: u32, modifiers: u8) {
    BRIDGE.with(|b| {
        if let Some(ref bridge) = *b.borrow() {
            let mut proxy = bridge.input_proxy.borrow_mut();
            bridge.with_shared_buffer(|buffer| {
                proxy.handle_key_up(key_code, modifiers, buffer);
            });
        }
    });
}

/// 处理文本输入事件（由平台层调用）
pub fn handle_text_input(text: &str) {
    BRIDGE.with(|b| {
        if let Some(ref bridge) = *b.borrow() {
            let mut proxy = bridge.input_proxy.borrow_mut();
            bridge.with_shared_buffer(|buffer| {
                proxy.handle_text_input(text, buffer);
            });
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
uniffi::setup_scaffolding!("dyxel_core");
