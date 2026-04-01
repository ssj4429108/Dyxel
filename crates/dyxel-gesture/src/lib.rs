// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! # Dyxel Gesture System
//! 
//! A robust gesture recognition system inspired by Flutter's GestureArena.
//! 
//! ## Architecture
//! 
//! Raw Touch Events → GestureArena → GestureRecognizers → GestureEvents → WASM
//! 
//! ## Key Concepts
//! 
//! - **GestureArena**: Manages competing gesture recognizers, resolves conflicts
//! - **GestureRecognizer**: Base trait for all gesture recognizers
//! - **GestureEvent**: High-level gesture events dispatched to WASM
//! - **GestureArenaMember**: Individual recognizer participating in arena
//! 
//! ## Supported Gestures
//! 
//! - **Tap**: Single tap, double tap, triple tap
//! - **LongPress**: Press and hold
//! - **Pan**: Drag with single or multiple pointers
//! - **Scale** (future): Pinch to zoom
//! - **Rotation** (future): Two-finger rotation

mod arena;
mod recognizer;
mod events;
mod hit_test;
mod spatial_hit_tester;

pub use arena::{
    GestureArena, 
    GestureArenaManager, 
    ArenaId, 
};

/// Gesture types supported by the system
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureType {
    Tap,
    DoubleTap,
    LongPress,
    Pan,
}
pub use recognizer::{
    GestureRecognizer, 
    RecognizerState,
    TapGestureRecognizer,
    LongPressGestureRecognizer, 
    PanGestureRecognizer,
    GestureConfig,
};
pub use events::{GestureEvent, GestureEventType, PointerEvent, PointerData};
pub use hit_test::{HitTestResult, HitTester, NoOpHitTester, RectHitTester, LayoutHitTester};
pub use spatial_hit_tester::{SpatialHitTester, SpatialStats};

use std::collections::HashMap;
use dyxel_shared::RawInputEvent;

/// Global gesture configuration
/// 
/// Values are aligned with Flutter's gesture constants:
/// https://api.flutter.dev/flutter/gestures/gestures-library.html
#[derive(Debug, Clone, Copy)]
pub struct GestureSettings {
    /// Maximum duration for a tap (milliseconds)
    /// Flutter: kPressTimeout (100ms) for tap down, but we use 300ms for tap up
    pub tap_timeout_ms: u64,
    /// Maximum movement for a tap (logical pixels)
    /// Flutter: kDoubleTapTouchSlop = 18.0
    pub tap_slop: f32,
    /// Duration for long press (milliseconds)
    /// Flutter: kLongPressTimeout = 500ms
    pub long_press_timeout_ms: u64,
    /// Maximum movement for long press (logical pixels)
    /// Flutter: kLongPressTouchSlop = 18.0 (uses kTouchSlop)
    pub long_press_slop: f32,
    /// Minimum movement to start pan (logical pixels)
    /// Flutter: kPanSlop = 18.0 (uses kTouchSlop)
    pub pan_slop: f32,
    /// Double tap timeout (milliseconds)
    /// Flutter: kDoubleTapTimeout = 300ms
    pub double_tap_timeout_ms: u64,
    /// Maximum distance between taps for double tap (logical pixels)
    /// Flutter: kDoubleTapSlop = 100.0
    pub double_tap_slop: f32,
}

impl Default for GestureSettings {
    fn default() -> Self {
        Self {
            // Note: Flutter uses kPressTimeout=100ms for onTapDown, 
            // but we use 300ms for complete tap recognition (down+up)
            tap_timeout_ms: 300,
            tap_slop: 18.0,       // Flutter kDoubleTapTouchSlop (kTouchSlop = 18.0)
            long_press_timeout_ms: 500,  // Flutter kLongPressTimeout
            long_press_slop: 18.0,       // Flutter kTouchSlop
            pan_slop: 18.0,              // Flutter kTouchSlop
            double_tap_timeout_ms: 300,  // Flutter kDoubleTapTimeout
            double_tap_slop: 100.0,      // Flutter kDoubleTapSlop
        }
    }
}

/// Gesture router - main entry point for gesture processing
/// 
/// This is the primary interface that the Host layer uses to
/// route touch events through the gesture system.
/// Provider for node gesture configuration
/// 
/// Returns the list of gesture types that a node supports.
/// This is used to determine which recognizers to create for a node.
pub trait GestureProvider: Send {
    fn get_node_gestures(&self, node_id: u32) -> Vec<GestureType>;
    
    /// Get parent node ID for bubbling (return 0 if no parent)
    fn get_parent_node(&self, node_id: u32) -> u32 {
        // Default implementation: no bubbling
        let _ = node_id;
        0
    }
}

pub struct GestureRouter {
    /// Global gesture settings
    settings: GestureSettings,
    /// Arena manager for each pointer
    arena_manager: GestureArenaManager,
    /// Hit tester for finding target nodes
    hit_tester: Box<dyn HitTester>,
    /// Gesture provider for querying node configuration
    gesture_provider: Box<dyn GestureProvider>,
    /// Active pointers and their current arena
    active_pointers: HashMap<u32, PointerState>,
    /// Gesture event callback
    event_callback: Box<dyn FnMut(GestureEvent)>,
}

#[derive(Debug, Clone)]
struct PointerState {
    #[allow(dead_code)]
    pointer_id: u32,
    arena_id: u64,
    #[allow(dead_code)]
    current_node: u32,
    current_x: f32,
    current_y: f32,
}

impl GestureRouter {
    pub fn new<F>(
        settings: GestureSettings,
        hit_tester: Box<dyn HitTester>,
        gesture_provider: Box<dyn GestureProvider>,
        event_callback: F,
    ) -> Self 
    where
        F: FnMut(GestureEvent) + 'static,
    {
        Self {
            settings,
            arena_manager: GestureArenaManager::new(),
            hit_tester,
            gesture_provider,
            active_pointers: HashMap::new(),
            event_callback: Box::new(event_callback),
        }
    }

    /// Sync hit tester with data source
    /// 
    /// Call this once per frame before processing input events
    pub fn sync(&mut self) {
        self.hit_tester.sync();
        
        // Note: LongPress now triggers on pointer up, not on timeout
        // So we don't need to send synthetic events for deadline checking
        // The deadline_met flag will be checked on the next Move or Up event
    }
    
    /// Find a node with gesture support, bubbling up to ancestors if needed
    fn find_node_with_gestures(&self, start_node: u32) -> (u32, Vec<GestureType>) {
        let mut current_node = start_node;
        
        // Bubble up until we find a node with gestures or reach the root
        while current_node != 0 {
            let gestures = self.gesture_provider.get_node_gestures(current_node);
            if !gestures.is_empty() {
                return (current_node, gestures);
            }
            // Try parent node
            current_node = self.gesture_provider.get_parent_node(current_node);
        }
        
        (0, Vec::new())
    }

    /// Process a raw input event
    /// 
    /// This is called from the Host layer for each touch event
    pub fn handle_input_event(&mut self, event: &RawInputEvent) {
        let pointer_event = self.convert_to_pointer_event(event);
        
        match pointer_event.event_type {
            events::PointerEventType::Down => {
                self.handle_pointer_down(pointer_event);
            }
            events::PointerEventType::Move => {
                self.handle_pointer_move(pointer_event);
            }
            events::PointerEventType::Up => {
                self.handle_pointer_up(pointer_event);
            }
            events::PointerEventType::Cancel => {
                self.handle_pointer_cancel(pointer_event);
            }
        }
    }

    fn convert_to_pointer_event(&self, raw: &RawInputEvent) -> PointerEvent {
        use dyxel_shared::InputEventType;
        
        let event_type = match raw.get_event_type() {
            Some(InputEventType::PointerDown) => events::PointerEventType::Down,
            Some(InputEventType::PointerMove) => events::PointerEventType::Move,
            Some(InputEventType::PointerUp) => events::PointerEventType::Up,
            Some(InputEventType::PointerCancel) => events::PointerEventType::Cancel,
            _ => events::PointerEventType::Cancel,
        };

        PointerEvent {
            event_type,
            pointer_id: raw.pointer_id,
            timestamp_us: raw.timestamp,
            x: raw.x,
            y: raw.y,
            pressure: raw.pressure,
            target_node_id: raw.target_node_id,
        }
    }

    fn handle_pointer_down(&mut self, event: PointerEvent) {
        // Hit test to find target node
        let hit_result = self.hit_tester.hit_test(event.x, event.y);
        let target_node = hit_result.node_id;

        // Get gesture types supported by this node (with bubbling)
        // Try target node first, then bubble up to ancestors
        let (target_node, gesture_types) = self.find_node_with_gestures(target_node);
        
        if gesture_types.is_empty() {
            return;
        }

        // Create or get arena for this pointer with specific gesture types
        let arena_id = self.arena_manager.create_arena(
            event.pointer_id,
            target_node,
            self.settings,
            &gesture_types,
        );

        // Track pointer state
        self.active_pointers.insert(
            event.pointer_id,
            PointerState {
                pointer_id: event.pointer_id,
                arena_id: arena_id.as_u64(),
                current_node: target_node,
                current_x: event.x,
                current_y: event.y,
            },
        );

        // Route event to arena
        if let Some(events) = self.arena_manager.handle_pointer_event(arena_id, event) {
            self.dispatch_events(events);
        }
    }

    fn handle_pointer_move(&mut self, event: PointerEvent) {
        if let Some(state) = self.active_pointers.get_mut(&event.pointer_id) {
            state.current_x = event.x;
            state.current_y = event.y;
            let arena_id = ArenaId::new(state.arena_id);
            if let Some(events) = self.arena_manager.handle_pointer_event(arena_id, event) {
                self.dispatch_events(events);
            }
        }
    }

    fn handle_pointer_up(&mut self, event: PointerEvent) {
        if let Some(state) = self.active_pointers.remove(&event.pointer_id) {
            let arena_id = ArenaId::new(state.arena_id);
            if let Some(events) = self.arena_manager.handle_pointer_event(arena_id, event) {
                self.dispatch_events(events);
            }
            // Note: arena is NOT closed here - it may be in Delayed state for multi-tap
            // The arena will be cleaned up by handle_pointer_event when it resolves
            // or when a new gesture starts
        }
    }

    fn handle_pointer_cancel(&mut self, event: PointerEvent) {
        if let Some(state) = self.active_pointers.remove(&event.pointer_id) {
            let arena_id = ArenaId::new(state.arena_id);
            if let Some(events) = self.arena_manager.handle_pointer_event(arena_id, event) {
                self.dispatch_events(events);
            }
            self.arena_manager.close_arena(arena_id);
        }
    }

    fn dispatch_events(&mut self, events: Vec<GestureEvent>) {
        for event in events {
            (self.event_callback)(event);
        }
    }

    /// Update settings at runtime
    pub fn update_settings(&mut self, settings: GestureSettings) {
        self.settings = settings;
    }
}

/// Mock gesture provider for testing
#[cfg(test)]
struct TestGestureProvider;

#[cfg(test)]
impl GestureProvider for TestGestureProvider {
    fn get_node_gestures(&self, _node_id: u32) -> Vec<GestureType> {
        vec![GestureType::Tap]
    }
}

/// Create a default gesture router with no-op callback (for testing)
#[cfg(test)]
impl Default for GestureRouter {
    fn default() -> Self {
        Self::new(
            GestureSettings::default(),
            Box::new(hit_test::NoOpHitTester),
            Box::new(TestGestureProvider),
            |_| {},
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gesture_settings_default() {
        let settings = GestureSettings::default();
        assert_eq!(settings.tap_timeout_ms, 300);
        assert_eq!(settings.tap_slop, 18.0);
    }

    /// Mock gesture provider for testing
    struct MockGestureProvider;
    
    impl GestureProvider for MockGestureProvider {
        fn get_node_gestures(&self, _node_id: u32) -> Vec<GestureType> {
            vec![GestureType::Tap]
        }
    }

    #[test]
    fn test_pointer_event_conversion() {
        let settings = GestureSettings::default();
        let router = GestureRouter::new(
            settings,
            Box::new(hit_test::NoOpHitTester),
            Box::new(MockGestureProvider),
            |_| {},
        );

        let raw = RawInputEvent {
            timestamp: 1000,
            pointer_id: 0,
            event_type: 0, // PointerDown
            _padding: [0; 3],
            x: 100.0,
            y: 200.0,
            pressure: 1.0,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: 5,
            flags: 0,
        };

        let pointer = router.convert_to_pointer_event(&raw);
        assert!(matches!(pointer.event_type, events::PointerEventType::Down));
        assert_eq!(pointer.x, 100.0);
        assert_eq!(pointer.y, 200.0);
    }
}
