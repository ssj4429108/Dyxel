// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Handler Registry - Tracks which nodes have gesture handlers
//!
//! This allows the Host to perform event bubbling without WASM involvement.
//! WASM notifies the Host when handlers are registered/unregistered.

use std::collections::HashSet;

/// Types of gesture handlers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandlerType {
    Tap,
    DoubleTap,
    LongPress,
    Pan,
}

/// Registry of gesture handlers per node
pub struct HandlerRegistry {
    tap_handlers: HashSet<u32>,
    double_tap_handlers: HashSet<u32>,
    long_press_handlers: HashSet<u32>,
    pan_handlers: HashSet<u32>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self {
            tap_handlers: HashSet::new(),
            double_tap_handlers: HashSet::new(),
            long_press_handlers: HashSet::new(),
            pan_handlers: HashSet::new(),
        }
    }

    /// Register a handler for a node
    pub fn register(&mut self, node_id: u32, handler_type: HandlerType) {
        match handler_type {
            HandlerType::Tap => self.tap_handlers.insert(node_id),
            HandlerType::DoubleTap => self.double_tap_handlers.insert(node_id),
            HandlerType::LongPress => self.long_press_handlers.insert(node_id),
            HandlerType::Pan => self.pan_handlers.insert(node_id),
        };
    }

    /// Unregister all handlers for a node
    pub fn unregister(&mut self, node_id: u32) {
        self.tap_handlers.remove(&node_id);
        self.double_tap_handlers.remove(&node_id);
        self.long_press_handlers.remove(&node_id);
        self.pan_handlers.remove(&node_id);
    }

    /// Check if a node has a specific handler
    pub fn has_handler(&self, node_id: u32, handler_type: HandlerType) -> bool {
        match handler_type {
            HandlerType::Tap => self.tap_handlers.contains(&node_id),
            HandlerType::DoubleTap => self.double_tap_handlers.contains(&node_id),
            HandlerType::LongPress => self.long_press_handlers.contains(&node_id),
            HandlerType::Pan => self.pan_handlers.contains(&node_id),
        }
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

    /// Get stats for debugging
    pub fn stats(&self) -> HandlerStats {
        HandlerStats {
            tap_count: self.tap_handlers.len(),
            double_tap_count: self.double_tap_handlers.len(),
            long_press_count: self.long_press_handlers.len(),
            pan_count: self.pan_handlers.len(),
        }
    }
}

/// Statistics for handler registry
#[derive(Debug, Clone, Copy)]
pub struct HandlerStats {
    pub tap_count: usize,
    pub double_tap_count: usize,
    pub long_press_count: usize,
    pub pan_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handler_registry() {
        let mut registry = HandlerRegistry::new();

        // Register handlers
        registry.register(10, HandlerType::Tap);
        registry.register(20, HandlerType::LongPress);
        registry.register(30, HandlerType::Pan);

        // Check individual handlers
        assert!(registry.has_handler(10, HandlerType::Tap));
        assert!(!registry.has_handler(10, HandlerType::LongPress));

        // Find handler in bubble path
        let path = vec![5, 8, 10, 20]; // Leaf to root
        assert_eq!(
            registry.find_handler(&path, HandlerType::Tap),
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
        assert!(!registry.has_handler(10, HandlerType::Tap));
    }
}
