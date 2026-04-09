// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Keyboard integration for Vello renderer
//!
//! This module provides keyboard offset functionality for keyboard avoidance.
//! The offset is applied at the root transform level to shift content up
//! when the soft keyboard appears.

use std::cell::RefCell;

thread_local! {
    static KEYBOARD_OFFSET: RefCell<f32> = RefCell::new(0.0);
}

/// Get the current keyboard avoidance offset
///
/// Returns a negative value (in logical pixels) that should be applied
/// to the root transform to shift content up when the keyboard appears.
pub fn keyboard_offset() -> f32 {
    KEYBOARD_OFFSET.with(|o| *o.borrow())
}

/// Set the keyboard avoidance offset
///
/// This is called from the host when the keyboard shows/hides.
/// A negative offset shifts content up, 0 means no offset.
pub fn set_keyboard_offset(offset: f32) {
    KEYBOARD_OFFSET.with(|o| {
        *o.borrow_mut() = offset;
    });
}

/// Reset keyboard offset to 0 (keyboard hidden)
pub fn reset_keyboard_offset() {
    KEYBOARD_OFFSET.with(|o| {
        *o.borrow_mut() = 0.0;
    });
}
