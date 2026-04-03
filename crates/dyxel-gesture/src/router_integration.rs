// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration layer that provides a Flutter-style GestureRouter compatible with dyxel-core.
//!
//! This implementation uses the gesture arena and recognizers for proper gesture competition.

use std::collections::HashMap;
use std::time::Instant;

use crate::events::{PointerEvent, PointerEventType, GestureEvent};
use crate::arena::{GestureArenaManager, GestureArena};
use crate::recognizer::{
    TapGestureRecognizer, LongPressGestureRecognizer,
    PanGestureRecognizer, ScaleGestureRecognizer
};

/// Gesture type for configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureType {
    Tap,
    LongPress,
    Pan,
    Scale,
    Rotation,
}

/// Gesture configuration for a node
#[derive(Debug, Clone)]
pub struct GestureConfig {
    /// Node ID this gesture belongs to
    pub node_id: u32,
    /// Registered gesture types for this node
    pub registered_types: Vec<GestureType>,
    /// Maximum tap count (1 for single, 2 for double, 3 for triple, etc.)
    pub max_tap_count: u32,
    /// Touch slop threshold
    pub slop: f32,
    /// Long press timeout
    pub long_press_timeout: std::time::Duration,
    /// Multi-click gap timeout
    pub multi_click_gap: std::time::Duration,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            node_id: 0,
            registered_types: vec![GestureType::Tap],
            max_tap_count: 1,
            slop: 18.0,
            long_press_timeout: std::time::Duration::from_millis(500),
            multi_click_gap: std::time::Duration::from_millis(300),
        }
    }
}

/// Helper struct to create recognizers without borrowing issues
struct RecognizerFactory {
    next_recognizer_id: u32,
}

impl RecognizerFactory {
    fn new(start_id: u32) -> Self {
        Self {
            next_recognizer_id: start_id,
        }
    }

    fn create_recognizers(
        &mut self,
        node_id: u32,
        config: &GestureConfig,
        arena: &mut GestureArena,
    ) -> Vec<u32> {
        let mut recognizer_ids = Vec::new();

        // Create Tap recognizer with max tap count (supports single/double/triple/etc)
        // Unified tap handling - all tap counts use the same recognizer
        let has_tap = config.registered_types.contains(&GestureType::Tap);

        if has_tap {
            let rid = self.next_recognizer_id;
            self.next_recognizer_id += 1;

            // Use max_tap_count from config (supports single/double/triple/etc)
            let tap_count = config.max_tap_count.max(1);

            let tap_recognizer = TapGestureRecognizer::new(rid, node_id)
                .with_tap_count(tap_count);

            arena.add_member(Box::new(tap_recognizer));
            recognizer_ids.push(rid);
        }

        // Create LongPress recognizer
        if config.registered_types.contains(&GestureType::LongPress) {
            let rid = self.next_recognizer_id;
            self.next_recognizer_id += 1;

            let long_press_recognizer = LongPressGestureRecognizer::new(rid, node_id)
                .with_duration(config.long_press_timeout);

            arena.add_member(Box::new(long_press_recognizer));
            recognizer_ids.push(rid);
        }

        // Create Pan recognizer
        if config.registered_types.contains(&GestureType::Pan) {
            let rid = self.next_recognizer_id;
            self.next_recognizer_id += 1;

            let pan_recognizer = PanGestureRecognizer::new(rid, node_id)
                .with_slop(config.slop);

            arena.add_member(Box::new(pan_recognizer));
            recognizer_ids.push(rid);
        }

        // Create Scale recognizer
        if config.registered_types.contains(&GestureType::Scale) {
            let rid = self.next_recognizer_id;
            self.next_recognizer_id += 1;

            let scale_recognizer = ScaleGestureRecognizer::new(rid, node_id);

            arena.add_member(Box::new(scale_recognizer));
            recognizer_ids.push(rid);
        }

        recognizer_ids
    }
}

/// A Flutter-style high-level gesture router
pub struct GestureRouter {
    arena_manager: GestureArenaManager,
    node_configs: HashMap<u32, GestureConfig>,
    /// Track which recognizers belong to which node
    node_recognizers: HashMap<u32, Vec<u32>>, // node_id -> [recognizer_id]
    next_recognizer_id: u32,
}

impl GestureRouter {
    pub fn new() -> Self {
        Self {
            arena_manager: GestureArenaManager::new(),
            node_configs: HashMap::new(),
            node_recognizers: HashMap::new(),
            next_recognizer_id: 1,
        }
    }

    pub fn register_node_gestures(&mut self, node_id: u32, config: GestureConfig) {
        self.node_configs.insert(node_id, config);
    }

    /// Route a pointer event and return gesture events
    pub fn route_pointer_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        self.route_pointer_event_with_path(event, vec![event.target_node_id])
    }

    /// Route a pointer event with an explicit bubble path
    pub fn route_pointer_event_with_path(&mut self, event: &PointerEvent, bubble_path: Vec<u32>) -> Vec<GestureEvent> {
        // On pointer down, create a new arena and add recognizers for all nodes in path
        if event.event_type == PointerEventType::Down {
            // Collect node configs first to avoid borrow issues
            let nodes_to_register: Vec<(u32, GestureConfig)> = bubble_path
                .iter()
                .filter(|&&node_id| node_id != 0)
                .filter_map(|&node_id| {
                    self.node_configs.get(&node_id).map(|config| (node_id, config.clone()))
                })
                .collect();

            // Check if arena already exists for this pointer
            let arena_existed = self.arena_manager.pointer_to_arena.contains_key(&event.pointer_id);

            // First, ensure arena exists by calling get_or_create_arena
            let _ = self.arena_manager.get_or_create_arena(event.pointer_id, event.target_node_id);

            // Get the arena_id for this pointer
            if let Some(&arena_id) = self.arena_manager.pointer_to_arena.get(&event.pointer_id) {
                // Only add recognizers if this is a new arena
                // For multi-tap, the same recognizer handles all taps
                if !arena_existed {
                    // Create recognizer factory with current next_id
                    let mut factory = RecognizerFactory::new(self.next_recognizer_id);

                    // Add recognizers for each node
                    for (node_id, config) in nodes_to_register {
                        if let Some(arena) = self.arena_manager.get_arena_mut(arena_id) {
                            let ids = factory.create_recognizers(node_id, &config, arena);
                            if !ids.is_empty() {
                                self.node_recognizers.insert(node_id, ids);
                            }
                        }
                    }

                    // Update next_recognizer_id
                    self.next_recognizer_id = factory.next_recognizer_id;
                }
            }
        }

        // Process the event through the arena manager
        let events = self.arena_manager.handle_pointer_event(event);

        // Note: We don't close arena on up/cancel immediately
        // because recognizers may need to process timers (e.g., single tap waiting for double tap timeout)
        // Arena will be closed in tick() after all timers are processed

        events
    }

    /// Update timers and return any pending events
    /// Also closes and cleans up arenas that are resolved
    pub fn tick(&mut self, now: Instant) -> Vec<GestureEvent> {
        // First check timers to allow recognizers to complete
        let events = self.arena_manager.tick(now);

        // Close arenas where all members are resolved
        let arena_ids = self.arena_manager.arena_ids();
        for arena_id in arena_ids {
            if let Some(arena) = self.arena_manager.get_arena(arena_id) {
                if arena.all_members_resolved() && !arena.is_closed() {
                    if let Some(arena) = self.arena_manager.get_arena_mut(arena_id) {
                        arena.close();
                    }
                }
            }
        }

        // Cleanup closed arenas
        self.arena_manager.cleanup();

        events
    }
}
