// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::engine::RenderState;
use dyxel_render_api::{DeviceHandle, QueueHandle, SurfaceState};

pub fn render_frame(e: &mut RenderState, s: &mut dyn SurfaceState) {
    // Update cursor blink state before rendering (uses frame time for smooth blinking)
    update_cursor_blink();

    // Downcast context to get Vello-specific types
    // Note: For a truly backend-agnostic approach, we would need to store
    // device and queue handles in RenderState. For now, we downcast since
    // we know we're using Vello.
    if let Some(v_ctx) = e.context.downcast_ref::<vello::util::RenderContext>() {
        let device = &v_ctx.devices[0].device;
        let queue = &v_ctx.devices[0].queue;

        // Sync TextInput states to renderer before rendering
        sync_text_input_states(e);

        // Create handles for the abstract API
        let device_handle = DeviceHandle::new(device);
        let queue_handle = QueueHandle::new(queue);

        log::trace!(
            "renderer: Starting frame render, surface size: {}x{}",
            s.width(),
            s.height()
        );
        if let Err(err) = e
            .backend
            .render(device_handle, queue_handle, s, &e.shared_state)
        {
            log::error!("renderer: Render error: {:?}", err);
        }
    } else {
        log::error!("renderer: Failed to downcast RenderContext to Vello context");
    }
}

/// Generation tracking for text inputs to avoid unnecessary re-renders
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static LAST_TEXT_GENERATIONS: RefCell<HashMap<u32, u64>> = RefCell::new(HashMap::new());
}

/// Get current time in milliseconds (for cursor blinking)
fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Update cursor blink state for all focused text inputs
fn update_cursor_blink() {
    crate::text_input::update_cursor_blink(current_time_ms());
}

/// Sync TextInput states from TextInputManager to the renderer
/// Uses generation tracking to skip unchanged text inputs
fn sync_text_input_states(_e: &RenderState) {
    use crate::text_input::TextInputManager;
    use dyxel_render_api::TextInputRenderState;

    LAST_TEXT_GENERATIONS.with(|generations| {
        let mut gens = generations.borrow_mut();

        // Get all active text input states
        TextInputManager::with(|manager| {
            // Get list of active node IDs
            let active_ids = manager.active_node_ids();
            log::debug!("sync_text_input_states: active_ids={:?}", active_ids);

            // Clean up generations for removed inputs
            gens.retain(|id, _| active_ids.contains(id));

            for node_id in &active_ids {
                if let Some(state) = manager.get(*node_id) {
                    // Check if state has changed (generation mismatch)
                    let last_gen = gens.get(node_id).copied();

                    // Sync if:
                    // 1. It's the first time we see this node (last_gen is None)
                    // 2. Generation has increased
                    // 3. It's focused (to keep cursor blinking smooth)
                    let needs_sync = last_gen.is_none()
                        || last_gen.unwrap() != state.generation
                        || state.focused;

                    if needs_sync {
                        let render_state = TextInputRenderState {
                            focused: state.focused,
                            text: state.text.clone(),
                            cursor_pos: state.cursor_pos,
                            selection_start: state.selection_start,
                            cursor_visible: state.cursor_visible,
                            secure: state.secure,
                            composing_text: state.composing_text.clone(),
                            is_composing: state.is_composing(),
                            composition_start: state
                                .cursor_pos
                                .saturating_sub(state.composing_text.len()),
                            placeholder: state.placeholder.clone(),
                        };

                        // Update renderer state via global function
                        dyxel_render_vello::update_text_input_state_global(*node_id, render_state);

                        // Track generation
                        gens.insert(*node_id, state.generation);
                    }
                }
            }
        });
    });
}
