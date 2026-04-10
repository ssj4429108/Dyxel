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

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::Sender;

/// Bridge handle for accessing input proxy and shared state
/// Used by platform layers to send input events
pub struct BridgeHandle {
    shared_state: SharedPtr<SharedMutex<SharedState>>,
    input_proxy: RefCell<InputProxy>,
    /// Sender for sending messages to the Logic thread
    #[cfg(not(target_arch = "wasm32"))]
    logic_tx: RefCell<Option<Sender<crate::bridge::LogicMessage>>>,
}

impl BridgeHandle {
    pub fn new(shared_state: SharedPtr<SharedMutex<SharedState>>) -> Self {
        let input_proxy = InputProxy::new(InputProxyConfig::default());
        Self {
            shared_state,
            input_proxy: RefCell::new(input_proxy),
            #[cfg(not(target_arch = "wasm32"))]
            logic_tx: RefCell::new(None),
        }
    }

    /// Set the logic thread sender (called after bridge initialization)
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_logic_tx(&self, tx: Sender<crate::bridge::LogicMessage>) {
        *self.logic_tx.borrow_mut() = Some(tx);
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
    log::info!("[BRIDGE] init_bridge called");
    BRIDGE.with(|b| {
        // Check if bridge already exists and has logic_tx set
        #[cfg(not(target_arch = "wasm32"))]
        let existing_logic_tx = b.borrow().as_ref().and_then(|bridge| {
            bridge.logic_tx.borrow().clone()
        });

        *b.borrow_mut() = Some(BridgeHandle::new(shared_state));

        // Restore logic_tx if it was previously set
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = existing_logic_tx {
            if let Some(ref bridge) = *b.borrow() {
                *bridge.logic_tx.borrow_mut() = Some(tx);
                log::info!("[BRIDGE] Restored existing logic_tx");
            }
        }

        log::info!("[BRIDGE] Bridge initialized successfully");
    });
}

/// Set the logic thread sender for the bridge (called after logic thread is created)
#[cfg(not(target_arch = "wasm32"))]
pub fn set_bridge_logic_tx(tx: Sender<crate::bridge::LogicMessage>) {
    log::info!("[BRIDGE] set_bridge_logic_tx called");
    BRIDGE.with(|b| {
        if let Some(ref bridge) = *b.borrow() {
            bridge.set_logic_tx(tx);
            log::info!("[BRIDGE] Logic tx set successfully");
        } else {
            log::warn!("[BRIDGE] set_bridge_logic_tx: bridge not initialized");
        }
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
    log::info!("[KEYBOARD] handle_text_input called: text='{}', len={}", text, text.len());
    BRIDGE.with(|b| {
        log::info!("[KEYBOARD] BRIDGE.with entered");
        let bridge_opt = b.borrow();
        log::info!("[KEYBOARD] BRIDGE borrowed, is_some={}", bridge_opt.is_some());
        if let Some(ref bridge) = *bridge_opt {
            log::info!("[KEYBOARD] Bridge exists");

            // Get the focused text input node_id
            let focused_id = text_input::focused_id();
            log::info!("[KEYBOARD] Focused text input id: {}", focused_id);

            if focused_id == 0 {
                log::warn!("[KEYBOARD] No focused text input, ignoring text input");
                return;
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                // Send TextInput message to Logic thread
                let tx_opt = bridge.logic_tx.borrow();
                if let Some(ref tx) = *tx_opt {
                    let msg = crate::bridge::LogicMessage::TextInput {
                        node_id: focused_id,
                        text: text.to_string(),
                    };
                    match tx.send(msg) {
                        Ok(_) => log::info!("[KEYBOARD] Sent TextInput message to logic thread"),
                        Err(e) => log::error!("[KEYBOARD] Failed to send TextInput message: {:?}", e),
                    }
                } else {
                    log::warn!("[KEYBOARD] logic_tx not set, falling back to direct buffer access");
                    // Fallback: try to access shared buffer directly
                    let mut proxy = bridge.input_proxy.borrow_mut();
                    bridge.with_shared_buffer(|buffer| {
                        proxy.handle_text_input(text, buffer);
                    });
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                // WASM: direct buffer access
                let mut proxy = bridge.input_proxy.borrow_mut();
                bridge.with_shared_buffer(|buffer| {
                    proxy.handle_text_input(text, buffer);
                });
            }
        } else {
            log::warn!("[KEYBOARD] handle_text_input: bridge not initialized");
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
uniffi::setup_scaffolding!("dyxel_core");
