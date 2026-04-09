// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Keyboard Manager - Handles keyboard state and keyboard avoidance

use std::cell::RefCell;

/// Keyboard state
#[derive(Debug, Clone, Copy)]
pub struct KeyboardState {
    /// Whether the keyboard is visible
    pub visible: bool,
    /// Keyboard height in logical pixels
    pub height: f32,
    /// Animation duration for keyboard show/hide (milliseconds)
    pub animation_duration_ms: u32,
    /// Screen height (for calculating offset)
    pub screen_height: f32,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self {
            visible: false,
            height: 0.0,
            animation_duration_ms: 250,
            screen_height: 0.0,
        }
    }
}

/// Keyboard avoidance configuration
#[derive(Debug, Clone, Copy)]
pub struct KeyboardAvoidConfig {
    /// Minimum padding between input and keyboard
    pub min_padding: f32,
    /// Whether keyboard avoidance is enabled
    pub enabled: bool,
}

impl Default for KeyboardAvoidConfig {
    fn default() -> Self {
        Self {
            min_padding: 20.0,
            enabled: true,
        }
    }
}

thread_local! {
    static KEYBOARD_STATE: RefCell<KeyboardState> = RefCell::new(KeyboardState::default());
    static AVOID_CONFIG: RefCell<KeyboardAvoidConfig> = RefCell::new(KeyboardAvoidConfig::default());
    static KEYBOARD_OFFSET: RefCell<f32> = RefCell::new(0.0);
}

/// Update keyboard state when keyboard shows/hides
pub fn update_keyboard_state(visible: bool, height: f32, animation_duration_ms: u32) {
    KEYBOARD_STATE.with(|state| {
        let mut s = state.borrow_mut();
        s.visible = visible;
        s.height = height;
        s.animation_duration_ms = animation_duration_ms;
    });

    if !visible {
        // Reset offset when keyboard hides
        set_keyboard_offset(0.0);
    }
}

/// Set screen height (needed for calculating offset)
pub fn set_screen_height(height: f32) {
    KEYBOARD_STATE.with(|state| {
        state.borrow_mut().screen_height = height;
    });
}

/// Get current keyboard state
pub fn keyboard_state() -> KeyboardState {
    KEYBOARD_STATE.with(|state| *state.borrow())
}

/// Check if keyboard is visible
pub fn is_keyboard_visible() -> bool {
    KEYBOARD_STATE.with(|state| state.borrow().visible)
}

/// Get keyboard height
pub fn keyboard_height() -> f32 {
    KEYBOARD_STATE.with(|state| state.borrow().height)
}

/// Calculate and set keyboard avoidance offset
///
/// This should be called with the focused text input's frame (position and size)
/// Returns the calculated offset
pub fn calculate_avoidance_offset(input_y: f32, input_height: f32) -> f32 {
    if !AVOID_CONFIG.with(|c| c.borrow().enabled) {
        return 0.0;
    }

    let keyboard = keyboard_state();
    if !keyboard.visible || keyboard.height <= 0.0 {
        return 0.0;
    }

    let min_padding = AVOID_CONFIG.with(|c| c.borrow().min_padding);
    let keyboard_top = keyboard.screen_height - keyboard.height;
    let input_bottom = input_y + input_height;

    // Check if input is covered by keyboard
    if input_bottom + min_padding > keyboard_top {
        let offset = keyboard_top - input_bottom - min_padding;
        set_keyboard_offset(offset);
        offset
    } else {
        0.0
    }
}

/// Get current keyboard avoidance offset
pub fn keyboard_offset() -> f32 {
    KEYBOARD_OFFSET.with(|offset| *offset.borrow())
}

/// Set keyboard avoidance offset directly
pub fn set_keyboard_offset(offset: f32) {
    KEYBOARD_OFFSET.with(|o| {
        *o.borrow_mut() = offset;
    });

    // Sync offset to renderer for transform application
    dyxel_render_vello::keyboard::set_keyboard_offset(offset);
}

/// Configure keyboard avoidance
pub fn configure_keyboard_avoidance(config: KeyboardAvoidConfig) {
    AVOID_CONFIG.with(|c| {
        *c.borrow_mut() = config;
    });
}

/// Enable/disable keyboard avoidance
pub fn set_keyboard_avoidance_enabled(enabled: bool) {
    AVOID_CONFIG.with(|c| {
        c.borrow_mut().enabled = enabled;
    });
}
