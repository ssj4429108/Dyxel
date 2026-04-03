// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Handler Registry - Tracks which nodes have gesture handlers
//!
//! This allows the Host to perform event bubbling without WASM involvement.
//! WASM notifies the Host when handlers are registered/unregistered.

use std::collections::HashMap;

/// Types of gesture handlers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandlerType {
    /// Tap with specific count (1=single, 2=double, 3=triple, etc.)
    Tap(u32),
    LongPress,
    Pan,
    Scale,
    Rotation,
}

/// Gesture type bitflags for RegisterGesture
pub const GESTURE_TAP: u16 = 1 << 0;
pub const GESTURE_LONG_PRESS: u16 = 1 << 1;
pub const GESTURE_PAN: u16 = 1 << 2;
pub const GESTURE_SCALE: u16 = 1 << 3;
pub const GESTURE_ROTATION: u16 = 1 << 4;

/// Config types for SetGestureConfig
pub const CONFIG_TAP_COUNT: u8 = 0;
pub const CONFIG_LONG_PRESS_TIMEOUT: u8 = 1;
pub const CONFIG_PAN_SLOP: u8 = 2;

impl HandlerType {
    /// Create a single tap handler type (for backward compatibility)
    pub fn single_tap() -> Self {
        Self::Tap(1)
    }

    /// Create a double tap handler type (for backward compatibility)
    pub fn double_tap() -> Self {
        Self::Tap(2)
    }
}

/// Registry of gesture handlers per node
///
/// For Tap handlers, stores the maximum tap count requested for each node.
/// This allows a single handler to be used for all tap counts up to that number.
pub struct HandlerRegistry {
    /// node_id -> max tap count (e.g., if both single and double tap are registered, stores 2)
    tap_handlers: HashMap<u32, u32>,
    long_press_handlers: HashMap<u32, ()>,
    pan_handlers: HashMap<u32, ()>,
    scale_handlers: HashMap<u32, ()>,
    rotation_handlers: HashMap<u32, ()>,
    /// Per-node gesture configuration (node_id -> (config_type, value))
    gesture_configs: HashMap<u32, HashMap<u8, u32>>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self {
            tap_handlers: HashMap::new(),
            long_press_handlers: HashMap::new(),
            pan_handlers: HashMap::new(),
            scale_handlers: HashMap::new(),
            rotation_handlers: HashMap::new(),
            gesture_configs: HashMap::new(),
        }
    }

    /// Register gestures by mask (unified API)
    pub fn register_by_mask(&mut self, node_id: u32, mask: u16) {
        if mask & GESTURE_TAP != 0 {
            // Default tap count = 1, can be upgraded via SetGestureConfig
            let current = self.tap_handlers.get(&node_id).copied().unwrap_or(0);
            if current < 1 {
                self.tap_handlers.insert(node_id, 1);
            }
        }
        if mask & GESTURE_LONG_PRESS != 0 {
            self.long_press_handlers.insert(node_id, ());
        }
        if mask & GESTURE_PAN != 0 {
            self.pan_handlers.insert(node_id, ());
        }
        if mask & GESTURE_SCALE != 0 {
            self.scale_handlers.insert(node_id, ());
        }
        if mask & GESTURE_ROTATION != 0 {
            self.rotation_handlers.insert(node_id, ());
        }
    }

    /// Set gesture configuration for a node
    pub fn set_config(&mut self, node_id: u32, config_type: u8, value: u32) {
        match config_type {
            CONFIG_TAP_COUNT => {
                // Update max tap count
                let current = self.tap_handlers.get(&node_id).copied().unwrap_or(0);
                if value > current {
                    self.tap_handlers.insert(node_id, value);
                }
            }
            _ => {
                // Store other configs
                self.gesture_configs
                    .entry(node_id)
                    .or_default()
                    .insert(config_type, value);
            }
        }
    }

    /// Get gesture configuration
    pub fn get_config(&self, node_id: u32, config_type: u8) -> Option<u32> {
        match config_type {
            CONFIG_TAP_COUNT => self.tap_handlers.get(&node_id).copied(),
            _ => self.gesture_configs.get(&node_id)?.get(&config_type).copied(),
        }
    }

    /// Register a handler for a node
    ///
    /// For Tap handlers, updates the max tap count if a higher count is requested.
    pub fn register(&mut self, node_id: u32, handler_type: HandlerType) {
        match handler_type {
            HandlerType::Tap(count) => {
                // Store the maximum tap count requested for this node
                let current = self.tap_handlers.get(&node_id).copied().unwrap_or(0);
                if count > current {
                    self.tap_handlers.insert(node_id, count);
                }
            }
            HandlerType::LongPress => {
                self.long_press_handlers.insert(node_id, ());
            }
            HandlerType::Pan => {
                self.pan_handlers.insert(node_id, ());
            }
            HandlerType::Scale => {
                self.scale_handlers.insert(node_id, ());
            }
            HandlerType::Rotation => {
                self.rotation_handlers.insert(node_id, ());
            }
        };
    }

    /// Unregister all handlers for a node
    pub fn unregister(&mut self, node_id: u32) {
        self.tap_handlers.remove(&node_id);
        self.long_press_handlers.remove(&node_id);
        self.pan_handlers.remove(&node_id);
        self.scale_handlers.remove(&node_id);
        self.rotation_handlers.remove(&node_id);
        self.gesture_configs.remove(&node_id);
    }

    /// Check if a node has a specific handler
    pub fn has_handler(&self, node_id: u32, handler_type: HandlerType) -> bool {
        match handler_type {
            HandlerType::Tap(count) => {
                // Node has a tap handler if max count >= requested count
                self.tap_handlers.get(&node_id).copied().unwrap_or(0) >= count
            }
            HandlerType::LongPress => self.long_press_handlers.contains_key(&node_id),
            HandlerType::Pan => self.pan_handlers.contains_key(&node_id),
            HandlerType::Scale => self.scale_handlers.contains_key(&node_id),
            HandlerType::Rotation => self.rotation_handlers.contains_key(&node_id),
        }
    }

    /// Get the maximum tap count registered for a node
    pub fn get_max_tap_count(&self, node_id: u32) -> u32 {
        self.tap_handlers.get(&node_id).copied().unwrap_or(0)
    }

    /// Check if node has any tap handler (regardless of count)
    pub fn has_any_tap_handler(&self, node_id: u32) -> bool {
        self.tap_handlers.contains_key(&node_id)
    }

    /// Find first node in bubble path with handler
    ///
    /// Returns the node_id that should handle the event, or None if no handler found.
    pub fn find_handler(&self, bubble_path: &[u32], handler_type: HandlerType) -> Option<u32> {
        for &node_id in bubble_path {
            if self.has_handler(node_id, handler_type) {
                return Some(node_id);
            }
        }
        None
    }

    /// Find first node in bubble path with any tap handler
    pub fn find_tap_handler(&self, bubble_path: &[u32]) -> Option<u32> {
        for &node_id in bubble_path {
            if self.has_any_tap_handler(node_id) {
                return Some(node_id);
            }
        }
        None
    }

    /// Get all gesture types registered for a node (for V2 integration)
    pub fn get_node_gestures(&self, node_id: u32) -> Vec<HandlerType> {
        let mut gestures = Vec::new();
        if let Some(count) = self.tap_handlers.get(&node_id) {
            gestures.push(HandlerType::Tap(*count));
        }
        if self.long_press_handlers.contains_key(&node_id) {
            gestures.push(HandlerType::LongPress);
        }
        if self.pan_handlers.contains_key(&node_id) {
            gestures.push(HandlerType::Pan);
        }
        if self.scale_handlers.contains_key(&node_id) {
            gestures.push(HandlerType::Scale);
        }
        if self.rotation_handlers.contains_key(&node_id) {
            gestures.push(HandlerType::Rotation);
        }
        gestures
    }

    /// Get stats for debugging
    pub fn stats(&self) -> HandlerStats {
        HandlerStats {
            tap_count: self.tap_handlers.len(),
            long_press_count: self.long_press_handlers.len(),
            pan_count: self.pan_handlers.len(),
            scale_count: self.scale_handlers.len(),
            rotation_count: self.rotation_handlers.len(),
        }
    }

    /// Get all tap handlers with their max counts
    pub fn tap_handlers(&self) -> &HashMap<u32, u32> {
        &self.tap_handlers
    }

    /// Get all long press handlers
    pub fn long_press_handlers(&self) -> &HashMap<u32, ()> {
        &self.long_press_handlers
    }

    /// Get all pan handlers
    pub fn pan_handlers(&self) -> &HashMap<u32, ()> {
        &self.pan_handlers
    }

    /// Get all scale handlers
    pub fn scale_handlers(&self) -> &HashMap<u32, ()> {
        &self.scale_handlers
    }

    /// Get all rotation handlers
    pub fn rotation_handlers(&self) -> &HashMap<u32, ()> {
        &self.rotation_handlers
    }
}

/// Statistics for handler registry
#[derive(Debug, Clone, Copy)]
pub struct HandlerStats {
    pub tap_count: usize,
    pub long_press_count: usize,
    pub pan_count: usize,
    pub scale_count: usize,
    pub rotation_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handler_registry() {
        let mut registry = HandlerRegistry::new();

        // Register handlers
        registry.register(10, HandlerType::Tap(1));
        registry.register(20, HandlerType::LongPress);
        registry.register(30, HandlerType::Pan);

        // Check individual handlers
        assert!(registry.has_handler(10, HandlerType::Tap(1)));
        assert!(!registry.has_handler(10, HandlerType::LongPress));

        // Find handler in bubble path
        let path = vec![5, 8, 10, 20]; // Leaf to root
        assert_eq!(
            registry.find_handler(&path, HandlerType::Tap(1)),
            Some(10)
        );
        assert_eq!(
            registry.find_handler(&path, HandlerType::LongPress),
            Some(20)
        );
        assert_eq!(
            registry.find_handler(&path, HandlerType::Pan),
            None
        );

        // Unregister
        registry.unregister(10);
        assert!(!registry.has_handler(10, HandlerType::Tap(1)));
    }

    #[test]
    fn test_multi_tap_registration() {
        let mut registry = HandlerRegistry::new();

        // Register single tap
        registry.register(1, HandlerType::Tap(1));
        assert_eq!(registry.get_max_tap_count(1), 1);
        assert!(registry.has_handler(1, HandlerType::Tap(1)));
        assert!(!registry.has_handler(1, HandlerType::Tap(2)));

        // Register double tap on same node - should upgrade to count 2
        registry.register(1, HandlerType::Tap(2));
        assert_eq!(registry.get_max_tap_count(1), 2);
        assert!(registry.has_handler(1, HandlerType::Tap(1)));
        assert!(registry.has_handler(1, HandlerType::Tap(2)));
        assert!(!registry.has_handler(1, HandlerType::Tap(3)));

        // Register triple tap - should upgrade to count 3
        registry.register(1, HandlerType::Tap(3));
        assert_eq!(registry.get_max_tap_count(1), 3);
        assert!(registry.has_handler(1, HandlerType::Tap(1)));
        assert!(registry.has_handler(1, HandlerType::Tap(2)));
        assert!(registry.has_handler(1, HandlerType::Tap(3)));
    }

    #[test]
    fn test_find_tap_handler() {
        let mut registry = HandlerRegistry::new();

        // Register double tap (which also covers single tap)
        registry.register(10, HandlerType::Tap(2));

        // find_tap_handler should find it
        let path = vec![5, 10];
        assert_eq!(registry.find_tap_handler(&path), Some(10));

        // Should work for both single and double tap queries
        assert!(registry.has_handler(10, HandlerType::Tap(1)));
        assert!(registry.has_handler(10, HandlerType::Tap(2)));
    }
}
