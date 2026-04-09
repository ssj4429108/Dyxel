// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TextInput Manager - Host-side text input lifecycle and state management

pub mod manager;

pub use manager::{
    copy, create_text_input, cut, focused_id, get, handle_menu_item, hide_context_menu,
    hide_keyboard, request_paste, set_cursor_position, set_focused, set_input_type, set_max_length,
    set_placeholder, set_return_key_type, set_secure, set_selection, set_text, show_context_menu,
    show_keyboard, update_cursor_blink, ClipboardIntegration, ContextMenuConfig,
    ContextMenuIntegration, ContextMenuItem, KeyboardIntegration, TextInputManager,
};

use dyxel_shared::TextInputState;
use std::collections::HashMap;

/// Registry of active text input instances
#[derive(Debug, Default)]
pub struct TextInputRegistry {
    /// Map of node_id -> TextInputState
    pub inputs: HashMap<u32, TextInputState>,
    /// Currently focused input node_id (0 = none)
    pub focused_id: u32,
}

impl TextInputRegistry {
    pub fn new() -> Self {
        Self {
            inputs: HashMap::new(),
            focused_id: 0,
        }
    }

    /// Create a new text input
    pub fn create(&mut self, node_id: u32) {
        let state = TextInputState::default();
        self.inputs.insert(node_id, state);
    }

    /// Remove a text input
    pub fn remove(&mut self, node_id: u32) {
        if self.focused_id == node_id {
            self.focused_id = 0;
        }
        self.inputs.remove(&node_id);
    }

    /// Get mutable reference to input state
    pub fn get_mut(&mut self, node_id: u32) -> Option<&mut TextInputState> {
        self.inputs.get_mut(&node_id)
    }

    /// Get immutable reference to input state
    pub fn get(&self, node_id: u32) -> Option<&TextInputState> {
        self.inputs.get(&node_id)
    }

    /// Set focused input
    pub fn set_focused(&mut self, node_id: u32, focused: bool) {
        if focused {
            // Unfocus previous
            if self.focused_id != 0 && self.focused_id != node_id {
                if let Some(prev) = self.inputs.get_mut(&self.focused_id) {
                    prev.focused = false;
                    prev.cursor_visible = false;
                }
            }
            self.focused_id = node_id;
            if let Some(state) = self.inputs.get_mut(&node_id) {
                state.focused = true;
                // Initialize cursor as visible and reset blink timer
                state.cursor_visible = true;
                state.last_blink_time = 0; // Will be updated on first blink tick
                state.generation = state.generation.wrapping_add(1);
            }
        } else if self.focused_id == node_id {
            self.focused_id = 0;
            if let Some(state) = self.inputs.get_mut(&node_id) {
                state.focused = false;
                state.cursor_visible = false;
                state.generation = state.generation.wrapping_add(1);
            }
        }
    }

    /// Check if any input is focused
    pub fn has_focused(&self) -> bool {
        self.focused_id != 0
    }

    /// Get the focused input ID
    pub fn focused_id(&self) -> u32 {
        self.focused_id
    }
}
