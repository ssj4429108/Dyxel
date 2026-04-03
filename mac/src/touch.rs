// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! macOS Trackpad Multi-Touch Support
//!
//! This module provides basic multi-touch tracking for macOS trackpads.
//! Note: Full multi-touch support requires intercepting NSEvent at the application level.

use std::collections::HashMap;

/// Tracks active touches and assigns stable pointer IDs
pub struct TouchTracker {
    /// Maps native touch identifier to our pointer ID
    active_touches: HashMap<u64, u32>,
    /// Next available pointer ID (starts at 1, 0 is reserved for mouse)
    next_id: u32,
}

impl TouchTracker {
    pub fn new() -> Self {
        Self {
            active_touches: HashMap::new(),
            next_id: 1,
        }
    }

    /// Get or create a pointer ID for a touch
    pub fn get_pointer_id(&mut self, native_id: u64) -> u32 {
        *self.active_touches.entry(native_id).or_insert_with(|| {
            let id = self.next_id;
            self.next_id += 1;
            // Wrap around if we somehow exceed reasonable limits
            if self.next_id > 100 {
                self.next_id = 1;
            }
            id
        })
    }

    /// Release a touch and return its pointer ID
    pub fn release_touch(&mut self, native_id: u64) -> Option<u32> {
        self.active_touches.remove(&native_id)
    }

    /// Check if any touches are currently active
    pub fn has_active_touches(&self) -> bool {
        !self.active_touches.is_empty()
    }

    #[allow(dead_code)]
    /// Get the number of active touches
    pub fn active_count(&self) -> usize {
        self.active_touches.len()
    }

    #[allow(dead_code)]
    /// Clear all tracked touches (e.g., on app background)
    pub fn clear(&mut self) {
        self.active_touches.clear();
        self.next_id = 1;
    }
}

/// Information about a touch event
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct TouchInfo {
    pub pointer_id: u32,
    pub x: f64,
    pub y: f64,
    pub phase: TouchPhase,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
    Cancelled,
}

/// Converts a native touch phase value to our enum
/// macOS touch phases: 0=Began, 1=Moved, 2=Stationary, 3=Ended, 4=Cancelled
#[allow(dead_code)]
pub fn convert_phase(native_phase: i32) -> TouchPhase {
    match native_phase {
        0 => TouchPhase::Began,
        1 | 2 => TouchPhase::Moved, // Treat stationary as moved (no change)
        3 => TouchPhase::Ended,
        _ => TouchPhase::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_touch_tracker() {
        let mut tracker = TouchTracker::new();

        // First touch gets ID 1
        assert_eq!(tracker.get_pointer_id(100), 1);
        assert_eq!(tracker.active_count(), 1);

        // Second touch gets ID 2
        assert_eq!(tracker.get_pointer_id(200), 2);
        assert_eq!(tracker.active_count(), 2);

        // Same native ID returns same pointer ID
        assert_eq!(tracker.get_pointer_id(100), 1);

        // Release first touch
        assert_eq!(tracker.release_touch(100), Some(1));
        assert_eq!(tracker.active_count(), 1);

        // New touch gets new ID (3, since we don't reuse)
        assert_eq!(tracker.get_pointer_id(300), 3);
    }
}
