// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TextInput Manager - Handles text input operations and platform integration

use super::TextInputRegistry;
use dyxel_shared::{InputType, ReturnKeyType, TextInputState};
use std::sync::{Arc, Mutex};

/// Platform-specific keyboard integration trait
pub trait KeyboardIntegration: Send + Sync {
    /// Show the on-screen keyboard
    fn show_keyboard(&self);
    /// Hide the on-screen keyboard
    fn hide_keyboard(&self);
    /// Update keyboard configuration
    fn configure(&self, input_type: InputType, return_key: ReturnKeyType, auto_correct: bool);
    /// Notify text changed (for IME)
    fn notify_text_changed(&self, text: &str, cursor_pos: usize);
}

/// Clipboard integration trait
pub trait ClipboardIntegration: Send + Sync {
    /// Copy text to clipboard
    fn copy(&self, text: &str);
    /// Get text from clipboard
    fn paste(&self) -> Option<String>;
}

/// TextInput Manager handles the lifecycle and operations of text inputs
pub struct TextInputManager {
    registry: TextInputRegistry,
    keyboard: Option<Box<dyn KeyboardIntegration>>,
    clipboard: Option<Box<dyn ClipboardIntegration>>,
    context_menu: Option<Box<dyn ContextMenuIntegration>>,
}

// Use Arc<Mutex<>> for cross-thread sharing (LogicThread creates, main thread renders)
lazy_static::lazy_static! {
    static ref MANAGER: Arc<Mutex<TextInputManager>> = Arc::new(Mutex::new(TextInputManager::new()));
}

impl TextInputManager {
    pub fn new() -> Self {
        Self {
            registry: TextInputRegistry::new(),
            keyboard: None,
            clipboard: None,
            context_menu: None,
        }
    }

    /// Access the global instance
    pub fn with<F, R>(f: F) -> R
    where
        F: FnOnce(&mut TextInputManager) -> R,
    {
        let mut manager = MANAGER.lock().unwrap();
        f(&mut manager)
    }

    /// Set keyboard integration
    pub fn set_keyboard_integration(&mut self, keyboard: Box<dyn KeyboardIntegration>) {
        self.keyboard = Some(keyboard);
    }

    /// Set clipboard integration
    pub fn set_clipboard_integration(&mut self, clipboard: Box<dyn ClipboardIntegration>) {
        self.clipboard = Some(clipboard);
    }

    // === Command Handlers (called from Runtime) ===

    /// Create a new text input
    pub fn create_text_input(&mut self, node_id: u32) {
        log::debug!(
            "Creating text input for node {} (before: registry.inputs.len()={})",
            node_id,
            self.registry.inputs.len()
        );
        self.registry.create(node_id);
        log::debug!(
            "Created text input for node {} (after: registry.inputs.len()={})",
            node_id,
            self.registry.inputs.len()
        );
    }

    /// Set input focus
    pub fn set_focused(&mut self, node_id: u32, focused: bool) {
        log::info!("Setting focus for node {} to {}", node_id, focused);
        self.registry.set_focused(node_id, focused);

        if let Some(keyboard) = &self.keyboard {
            if focused {
                if let Some(state) = self.registry.get(node_id) {
                    keyboard.configure(state.input_type, state.return_key_type, state.auto_correct);
                }
                keyboard.show_keyboard();
            } else if self.registry.focused_id() == 0 {
                keyboard.hide_keyboard();
            }
        }
    }

    /// Set text content
    pub fn set_text(&mut self, node_id: u32, text: String) {
        log::debug!("Setting text for node {}: '{}'", node_id, text);
        if let Some(state) = self.registry.get_mut(node_id) {
            state.set_text(text);

            if let Some(keyboard) = &self.keyboard {
                keyboard.notify_text_changed(&state.text, state.cursor_pos);
            }
        }
    }

    /// Set cursor position
    pub fn set_cursor_position(&mut self, node_id: u32, pos: u32) {
        log::debug!("Setting cursor for node {} to {}", node_id, pos);
        if let Some(state) = self.registry.get_mut(node_id) {
            state.cursor_pos = pos.min(state.text.len() as u32) as usize;
            state.selection_start = state.cursor_pos;
        }
    }

    /// Set selection range
    pub fn set_selection(&mut self, node_id: u32, start: u32, end: u32) {
        log::debug!("Setting selection for node {}: {}-{}", node_id, start, end);
        if let Some(state) = self.registry.get_mut(node_id) {
            let max = state.text.len() as u32;
            state.selection_start = start.min(max) as usize;
            state.cursor_pos = end.min(max) as usize;
        }
    }

    /// Set input type
    pub fn set_input_type(&mut self, node_id: u32, input_type: InputType) {
        log::debug!(
            "Setting input type for node {} to {:?}",
            node_id,
            input_type
        );
        if let Some(state) = self.registry.get_mut(node_id) {
            state.input_type = input_type;
        }
    }

    /// Set return key type
    pub fn set_return_key_type(&mut self, node_id: u32, key_type: ReturnKeyType) {
        log::debug!(
            "Setting return key type for node {} to {:?}",
            node_id,
            key_type
        );
        if let Some(state) = self.registry.get_mut(node_id) {
            state.return_key_type = key_type;
        }
    }

    /// Set placeholder
    pub fn set_placeholder(&mut self, node_id: u32, placeholder: String) {
        log::info!(
            "Setting placeholder for node {}: '{}'",
            node_id,
            placeholder
        );
        if let Some(state) = self.registry.get_mut(node_id) {
            state.placeholder = placeholder.clone();
            state.generation = state.generation.wrapping_add(1);
            log::info!(
                "Placeholder set for node {}: '{}' (len={})",
                node_id,
                placeholder,
                placeholder.len()
            );
        } else {
            log::warn!("Cannot set placeholder for node {}: not found", node_id);
        }
    }

    /// Set max length
    pub fn set_max_length(&mut self, node_id: u32, max_length: u32) {
        log::debug!("Setting max length for node {} to {}", node_id, max_length);
        if let Some(state) = self.registry.get_mut(node_id) {
            state.max_length = max_length;
        }
    }

    /// Set secure mode (password)
    pub fn set_secure(&mut self, node_id: u32, secure: bool) {
        log::debug!("Setting secure mode for node {} to {}", node_id, secure);
        if let Some(state) = self.registry.get_mut(node_id) {
            state.secure = secure;
        }
    }

    /// Show keyboard
    pub fn show_keyboard(&self) {
        log::debug!("Showing keyboard");
        if let Some(keyboard) = &self.keyboard {
            keyboard.show_keyboard();
        }
    }

    /// Hide keyboard
    pub fn hide_keyboard(&self) {
        log::debug!("Hiding keyboard");
        if let Some(keyboard) = &self.keyboard {
            keyboard.hide_keyboard();
        }
    }

    /// Copy selected text to clipboard
    pub fn copy(&self, node_id: u32) {
        if let Some(state) = self.registry.get(node_id) {
            if let Some(selected) = state.selected_text() {
                log::debug!("Copying text from node {}: '{}'", node_id, selected);
                if let Some(clipboard) = &self.clipboard {
                    clipboard.copy(&selected);
                }
            }
        }
    }

    /// Cut selected text to clipboard
    pub fn cut(&mut self, node_id: u32) {
        if let Some(state) = self.registry.get(node_id) {
            if let Some(selected) = state.selected_text() {
                log::debug!("Cutting text from node {}: '{}'", node_id, selected);
                if let Some(clipboard) = &self.clipboard {
                    clipboard.copy(&selected);
                }
            }
        }
        // TODO: Delete selected text
    }

    /// Request paste from clipboard
    pub fn request_paste(&self, node_id: u32) {
        log::debug!("Requesting paste for node {}", node_id);
        if let Some(clipboard) = &self.clipboard {
            if let Some(text) = clipboard.paste() {
                // TODO: Send event to WASM
                log::debug!("Pasted text: '{}'", text);
            }
        }
    }

    /// Remove a text input
    pub fn remove(&mut self, node_id: u32) {
        log::debug!("Removing text input for node {}", node_id);
        self.registry.remove(node_id);
    }

    // === Getters ===

    /// Get input state
    pub fn get(&self, node_id: u32) -> Option<&TextInputState> {
        self.registry.get(node_id)
    }

    /// Get mutable input state
    pub fn get_mut(&mut self, node_id: u32) -> Option<&mut TextInputState> {
        self.registry.get_mut(node_id)
    }

    /// Get focused input ID
    pub fn focused_id(&self) -> u32 {
        self.registry.focused_id()
    }

    /// Check if any input is focused
    pub fn has_focused(&self) -> bool {
        self.registry.has_focused()
    }

    /// Get all active text input node IDs
    pub fn active_node_ids(&self) -> Vec<u32> {
        let ids: Vec<u32> = self.registry.inputs.keys().cloned().collect();
        log::debug!(
            "active_node_ids: registry.inputs.len()={}, ids={:?}",
            self.registry.inputs.len(),
            ids
        );
        ids
    }

    /// Synchronize all text input states to the Vello renderer for cursor and selection rendering
    pub fn sync_to_renderer(&self) {
        for (&id, state) in self.registry.inputs.iter() {
            let render_state = dyxel_render_api::TextInputRenderState {
                focused: state.focused,
                text: state.text.clone(),
                cursor_pos: state.cursor_pos,
                selection_start: state.selection_start,
                cursor_visible: state.cursor_visible,
                secure: state.secure,
                composing_text: state.composing_text.clone(),
                is_composing: state.is_composing(),
                composition_start: state.cursor_pos.saturating_sub(state.composing_text.len()),
                placeholder: state.placeholder.clone(),
            };
            dyxel_render_vello::update_text_input_state_global(id, render_state);
        }
    }
}

impl Default for TextInputManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function to sync to renderer
pub fn sync_to_renderer() {
    TextInputManager::with(|m| m.sync_to_renderer());
}

/// Convenience function to create a text input
pub fn create_text_input(node_id: u32) {
    TextInputManager::with(|m| m.create_text_input(node_id));
}

/// Convenience function to set focus
pub fn set_focused(node_id: u32, focused: bool) {
    TextInputManager::with(|m| m.set_focused(node_id, focused));
}

/// Convenience function to set text
pub fn set_text(node_id: u32, text: String) {
    TextInputManager::with(|m| m.set_text(node_id, text));
}

/// Convenience function to set cursor position
pub fn set_cursor_position(node_id: u32, pos: u32) {
    TextInputManager::with(|m| m.set_cursor_position(node_id, pos));
}

/// Convenience function to set selection
pub fn set_selection(node_id: u32, start: u32, end: u32) {
    TextInputManager::with(|m| m.set_selection(node_id, start, end));
}

/// Convenience function to set input type
pub fn set_input_type(node_id: u32, input_type: InputType) {
    TextInputManager::with(|m| m.set_input_type(node_id, input_type));
}

/// Convenience function to set return key type
pub fn set_return_key_type(node_id: u32, key_type: ReturnKeyType) {
    TextInputManager::with(|m| m.set_return_key_type(node_id, key_type));
}

/// Convenience function to set placeholder
pub fn set_placeholder(node_id: u32, placeholder: String) {
    TextInputManager::with(|m| m.set_placeholder(node_id, placeholder));
}

/// Convenience function to set max length
pub fn set_max_length(node_id: u32, max_length: u32) {
    TextInputManager::with(|m| m.set_max_length(node_id, max_length));
}

/// Convenience function to set secure mode
pub fn set_secure(node_id: u32, secure: bool) {
    TextInputManager::with(|m| m.set_secure(node_id, secure));
}

/// Convenience function to show keyboard
pub fn show_keyboard() {
    TextInputManager::with(|m| m.show_keyboard());
}

/// Convenience function to hide keyboard
pub fn hide_keyboard() {
    TextInputManager::with(|m| m.hide_keyboard());
}

/// Convenience function to copy
pub fn copy(node_id: u32) {
    TextInputManager::with(|m| m.copy(node_id));
}

/// Convenience function to cut
pub fn cut(node_id: u32) {
    TextInputManager::with(|m| m.cut(node_id));
}

/// Convenience function to request paste
pub fn request_paste(node_id: u32) {
    TextInputManager::with(|m| m.request_paste(node_id));
}

/// Convenience function to get input state
pub fn get(node_id: u32) -> Option<TextInputState> {
    TextInputManager::with(|m| m.get(node_id).cloned())
}

/// Convenience function to get focused input ID
pub fn focused_id() -> u32 {
    TextInputManager::with(|m| m.focused_id())
}

// === Context Menu Integration ===

/// Menu item types for text input context menu
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContextMenuItem {
    SelectAll = 0,
    Copy = 1,
    Paste = 2,
    Cut = 3,
}

impl ContextMenuItem {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(ContextMenuItem::SelectAll),
            1 => Some(ContextMenuItem::Copy),
            2 => Some(ContextMenuItem::Paste),
            3 => Some(ContextMenuItem::Cut),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ContextMenuItem::SelectAll => "Select All",
            ContextMenuItem::Copy => "Copy",
            ContextMenuItem::Paste => "Paste",
            ContextMenuItem::Cut => "Cut",
        }
    }
}

/// Context menu configuration
#[derive(Debug, Clone)]
pub struct ContextMenuConfig {
    /// Menu items to show
    pub items: Vec<ContextMenuItem>,
    /// Position in screen coordinates (if None, centered on input)
    pub position: Option<(f32, f32)>,
}

impl Default for ContextMenuConfig {
    fn default() -> Self {
        Self {
            items: vec![
                ContextMenuItem::SelectAll,
                ContextMenuItem::Copy,
                ContextMenuItem::Paste,
                ContextMenuItem::Cut,
            ],
            position: None,
        }
    }
}

/// Platform-specific context menu integration trait
pub trait ContextMenuIntegration: Send + Sync {
    /// Show context menu at the specified position
    fn show_menu(&self, node_id: u32, items: &[ContextMenuItem], position: Option<(f32, f32)>);
    /// Hide context menu
    fn hide_menu(&self, node_id: u32);
}

impl TextInputManager {
    /// Set context menu integration
    pub fn set_context_menu_integration(&mut self, context_menu: Box<dyn ContextMenuIntegration>) {
        self.context_menu = Some(context_menu);
    }

    /// Show context menu for a text input
    pub fn show_context_menu(&self, node_id: u32, position: Option<(f32, f32)>) {
        if let Some(context_menu) = &self.context_menu {
            // Determine which items to show based on state
            let items = if let Some(state) = self.registry.get(node_id) {
                let mut items = Vec::new();

                // Always show Select All if there's text
                if !state.text.is_empty() {
                    items.push(ContextMenuItem::SelectAll);
                }

                // Show Copy/Cut only if text is selected
                if state.has_selection() {
                    items.push(ContextMenuItem::Copy);
                    if !state.read_only {
                        items.push(ContextMenuItem::Cut);
                    }
                }

                // Show Paste if clipboard has content (check via clipboard integration)
                // For now, always show Paste option
                if !state.read_only {
                    items.push(ContextMenuItem::Paste);
                }

                items
            } else {
                // Default menu if state not found
                vec![
                    ContextMenuItem::SelectAll,
                    ContextMenuItem::Copy,
                    ContextMenuItem::Paste,
                    ContextMenuItem::Cut,
                ]
            };

            context_menu.show_menu(node_id, &items, position);
        }
    }

    /// Hide context menu
    pub fn hide_context_menu(&self, node_id: u32) {
        if let Some(context_menu) = &self.context_menu {
            context_menu.hide_menu(node_id);
        }
    }

    /// Handle context menu item selection
    pub fn handle_menu_item(&mut self, node_id: u32, item: ContextMenuItem) {
        match item {
            ContextMenuItem::SelectAll => {
                if let Some(state) = self.registry.get_mut(node_id) {
                    state.select_all();
                }
            }
            ContextMenuItem::Copy => {
                self.copy(node_id);
            }
            ContextMenuItem::Cut => {
                self.cut(node_id);
            }
            ContextMenuItem::Paste => {
                self.request_paste(node_id);
            }
        }

        // Hide menu after selection
        self.hide_context_menu(node_id);
    }

    // === Cursor Blinking ===

    /// Update cursor blink state for all focused inputs
    /// Call this regularly (e.g., every frame or in a timer)
    pub fn update_cursor_blink(&mut self, current_time_ms: u64) {
        const BLINK_INTERVAL_MS: u64 = 530; // iOS-style blink interval

        for (_node_id, state) in self.registry.inputs.iter_mut() {
            if state.focused {
                // Initialize last_blink_time if not set (first focus)
                if state.last_blink_time == 0 {
                    state.last_blink_time = current_time_ms;
                    state.cursor_visible = true;
                    state.generation = state.generation.wrapping_add(1);
                    continue;
                }

                // Check if it's time to toggle cursor visibility
                if current_time_ms.saturating_sub(state.last_blink_time) >= BLINK_INTERVAL_MS {
                    state.cursor_visible = !state.cursor_visible;
                    state.last_blink_time = current_time_ms;
                    // Increment generation to trigger render update
                    state.generation = state.generation.wrapping_add(1);
                }
            } else {
                // Ensure cursor is visible when not focused (or hidden based on design)
                // iOS style: cursor is not shown when unfocused
                state.cursor_visible = false;
                state.last_blink_time = 0; // Reset for next focus
            }
        }
    }
}

/// Convenience function to show context menu
pub fn show_context_menu(node_id: u32, position: Option<(f32, f32)>) {
    TextInputManager::with(|m| m.show_context_menu(node_id, position));
}

/// Convenience function to hide context menu
pub fn hide_context_menu(node_id: u32) {
    TextInputManager::with(|m| m.hide_context_menu(node_id));
}

/// Convenience function to handle menu item selection
pub fn handle_menu_item(node_id: u32, item: u8) {
    if let Some(menu_item) = ContextMenuItem::from_u8(item) {
        TextInputManager::with(|m| m.handle_menu_item(node_id, menu_item));
    }
}

/// Convenience function to update cursor blink state
/// Call this regularly (e.g., every frame or 60fps timer)
pub fn update_cursor_blink(current_time_ms: u64) {
    TextInputManager::with(|m| m.update_cursor_blink(current_time_ms));
}
