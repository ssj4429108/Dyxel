// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test utilities for Gesture System V3
//!
//! Provides helper functions and builders for writing gesture tests.

use crate::events::{GestureEvent, GestureEventType, PointerEvent, PointerEventType};
use std::time::{Duration, Instant};

/// Builder for creating pointer events in tests
///
/// # Example
/// ```rust
/// let down = PointerEventBuilder::new(0).down(100.0, 100.0);
/// let move_evt = PointerEventBuilder::new(0).move_to(120.0, 120.0);
/// let up = PointerEventBuilder::new(0).up();
/// ```
pub struct PointerEventBuilder {
    pointer_id: u32,
    timestamp_us: u64,
    target_node_id: u32,
    pressure: f32,
}

impl PointerEventBuilder {
    /// Create a new builder with default settings
    pub fn new(pointer_id: u32) -> Self {
        Self {
            pointer_id,
            timestamp_us: 0,
            target_node_id: 1,
            pressure: 1.0,
        }
    }

    /// Set the target node ID
    pub fn node_id(mut self, node_id: u32) -> Self {
        self.target_node_id = node_id;
        self
    }

    /// Set the timestamp
    pub fn timestamp(mut self, timestamp_us: u64) -> Self {
        self.timestamp_us = timestamp_us;
        self
    }

    /// Set the pressure
    pub fn pressure(mut self, pressure: f32) -> Self {
        self.pressure = pressure;
        self
    }

    /// Create a PointerDown event
    pub fn down(self, x: f32, y: f32) -> PointerEvent {
        PointerEvent {
            event_type: PointerEventType::Down,
            pointer_id: self.pointer_id,
            timestamp_us: self.timestamp_us,
            x,
            y,
            pressure: self.pressure,
            target_node_id: self.target_node_id,
        }
    }

    /// Create a PointerMove event
    pub fn move_to(self, x: f32, y: f32) -> PointerEvent {
        PointerEvent {
            event_type: PointerEventType::Move,
            pointer_id: self.pointer_id,
            timestamp_us: self.timestamp_us,
            x,
            y,
            pressure: self.pressure,
            target_node_id: self.target_node_id,
        }
    }

    /// Create a PointerUp event
    pub fn up(self) -> PointerEvent {
        PointerEvent {
            event_type: PointerEventType::Up,
            pointer_id: self.pointer_id,
            timestamp_us: self.timestamp_us,
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            target_node_id: self.target_node_id,
        }
    }

    /// Create a PointerUp event at specific coordinates
    pub fn up_at(self, x: f32, y: f32) -> PointerEvent {
        PointerEvent {
            event_type: PointerEventType::Up,
            pointer_id: self.pointer_id,
            timestamp_us: self.timestamp_us,
            x,
            y,
            pressure: 0.0,
            target_node_id: self.target_node_id,
        }
    }

    /// Create a PointerCancel event
    pub fn cancel(self) -> PointerEvent {
        PointerEvent {
            event_type: PointerEventType::Cancel,
            pointer_id: self.pointer_id,
            timestamp_us: self.timestamp_us,
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            target_node_id: self.target_node_id,
        }
    }
}

/// Simulates complete gesture sequences for testing
///
/// # Example
/// ```rust
/// let events = GestureSimulator::tap(&mut router, 0, 100.0, 100.0);
/// let events = GestureSimulator::double_tap(&mut router, 0, 100.0, 100.0);
/// ```
pub struct GestureSimulator<R> {
    router: R,
    current_time: Instant,
}

/// Trait for gesture routers that can be used with GestureSimulator
pub trait SimulatableRouter {
    fn route_pointer_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent>;
    fn tick(&mut self, now: Instant) -> Vec<GestureEvent>;
}

impl<R: SimulatableRouter> GestureSimulator<R> {
    /// Create a new simulator
    pub fn new(router: R) -> Self {
        Self {
            router,
            current_time: Instant::now(),
        }
    }

    /// Advance the internal clock
    pub fn advance(&mut self, duration: Duration) {
        self.current_time += duration;
    }

    /// Get the current time
    pub fn now(&self) -> Instant {
        self.current_time
    }

    /// Simulate a single tap
    pub fn tap(&mut self, pointer_id: u32, node_id: u32, x: f32, y: f32) -> Vec<GestureEvent> {
        let mut events = vec![];

        let down = PointerEventBuilder::new(pointer_id)
            .node_id(node_id)
            .down(x, y);
        events.extend(self.router.route_pointer_event(&down));

        let up = PointerEventBuilder::new(pointer_id)
            .node_id(node_id)
            .up_at(x, y);
        events.extend(self.router.route_pointer_event(&up));

        events
    }

    /// Simulate a double tap
    pub fn double_tap(
        &mut self,
        pointer_id: u32,
        node_id: u32,
        x: f32,
        y: f32,
    ) -> Vec<GestureEvent> {
        let mut events = vec![];

        // First tap
        events.extend(self.tap(pointer_id, node_id, x, y));

        // Small delay (within double-tap window)
        self.advance(Duration::from_millis(100));

        // Second tap
        let down = PointerEventBuilder::new(pointer_id)
            .node_id(node_id)
            .down(x, y);
        events.extend(self.router.route_pointer_event(&down));

        let up = PointerEventBuilder::new(pointer_id)
            .node_id(node_id)
            .up_at(x, y);
        events.extend(self.router.route_pointer_event(&up));

        events
    }

    /// Simulate a triple tap
    pub fn triple_tap(
        &mut self,
        pointer_id: u32,
        node_id: u32,
        x: f32,
        y: f32,
    ) -> Vec<GestureEvent> {
        let mut events = vec![];

        events.extend(self.double_tap(pointer_id, node_id, x, y));
        self.advance(Duration::from_millis(100));
        events.extend(self.tap(pointer_id, node_id, x, y));

        events
    }

    /// Simulate a long press
    pub fn long_press(
        &mut self,
        pointer_id: u32,
        node_id: u32,
        x: f32,
        y: f32,
        duration: Duration,
    ) -> Vec<GestureEvent> {
        let mut events = vec![];

        let down = PointerEventBuilder::new(pointer_id)
            .node_id(node_id)
            .down(x, y);
        events.extend(self.router.route_pointer_event(&down));

        // Wait for long press timeout
        self.advance(duration);
        events.extend(self.router.tick(self.current_time));

        // Release
        let up = PointerEventBuilder::new(pointer_id)
            .node_id(node_id)
            .up_at(x, y);
        events.extend(self.router.route_pointer_event(&up));

        events
    }

    /// Simulate a pan gesture
    pub fn pan(
        &mut self,
        pointer_id: u32,
        node_id: u32,
        from: (f32, f32),
        to: (f32, f32),
        steps: u32,
    ) -> Vec<GestureEvent> {
        let mut events = vec![];

        // Down
        let down = PointerEventBuilder::new(pointer_id)
            .node_id(node_id)
            .down(from.0, from.1);
        events.extend(self.router.route_pointer_event(&down));

        // Move steps
        let dx = (to.0 - from.0) / steps as f32;
        let dy = (to.1 - from.1) / steps as f32;

        for i in 1..=steps {
            let move_evt = PointerEventBuilder::new(pointer_id)
                .node_id(node_id)
                .move_to(from.0 + dx * i as f32, from.1 + dy * i as f32);
            events.extend(self.router.route_pointer_event(&move_evt));
        }

        // Up
        let up = PointerEventBuilder::new(pointer_id)
            .node_id(node_id)
            .up_at(to.0, to.1);
        events.extend(self.router.route_pointer_event(&up));

        events
    }

    /// Simulate a scale gesture (two-finger pinch)
    pub fn scale(
        &mut self,
        node_id: u32,
        center: (f32, f32),
        initial_distance: f32,
        final_distance: f32,
        steps: u32,
    ) -> Vec<GestureEvent> {
        let mut events = vec![];
        let pointer1 = 0;
        let pointer2 = 1;

        // Initial positions
        let p1_start = (center.0 - initial_distance / 2.0, center.1);
        let p2_start = (center.0 + initial_distance / 2.0, center.1);

        // Down for both pointers
        let down1 = PointerEventBuilder::new(pointer1).node_id(node_id).down(p1_start.0, p1_start.1);
        events.extend(self.router.route_pointer_event(&down1));

        let down2 = PointerEventBuilder::new(pointer2).node_id(node_id).down(p2_start.0, p2_start.1);
        events.extend(self.router.route_pointer_event(&down2));

        // Move steps
        let distance_delta = (final_distance - initial_distance) / steps as f32;

        for i in 1..=steps {
            let current_distance = initial_distance + distance_delta * i as f32;
            let p1 = PointerEventBuilder::new(pointer1)
                .node_id(node_id)
                .move_to(center.0 - current_distance / 2.0, center.1);
            events.extend(self.router.route_pointer_event(&p1));

            let p2 = PointerEventBuilder::new(pointer2)
                .node_id(node_id)
                .move_to(center.0 + current_distance / 2.0, center.1);
            events.extend(self.router.route_pointer_event(&p2));
        }

        // Up for both pointers
        let up1 = PointerEventBuilder::new(pointer1).node_id(node_id).up();
        events.extend(self.router.route_pointer_event(&up1));

        let up2 = PointerEventBuilder::new(pointer2).node_id(node_id).up();
        events.extend(self.router.route_pointer_event(&up2));

        events
    }

    /// Wait for a specific duration and trigger timers
    pub fn wait(&mut self, duration: Duration) -> Vec<GestureEvent> {
        self.advance(duration);
        self.router.tick(self.current_time)
    }

    /// Process pending timers without advancing time
    pub fn process_timers(&mut self) -> Vec<GestureEvent> {
        self.router.tick(self.current_time)
    }
}

/// Assertions for gesture events
pub trait GestureEventAssertions {
    /// Assert that events contain exactly one tap with given count
    fn assert_tap(&self, count: u32) -> &Self;

    /// Assert that events contain long press start
    fn assert_long_press_start(&self) -> &Self;

    /// Assert that events contain long press end
    fn assert_long_press_end(&self) -> &Self;

    /// Assert that events contain pan start
    fn assert_pan_start(&self) -> &Self;

    /// Assert that events contain pan update with specific delta
    fn assert_pan_update(&self, delta_x: f32, delta_y: f32) -> &Self;

    /// Assert that events contain pan end
    fn assert_pan_end(&self) -> &Self;

    /// Assert that events contain scale start
    fn assert_scale_start(&self) -> &Self;

    /// Assert that events contain scale update with specific scale
    fn assert_scale_update(&self, scale: f32) -> &Self;

    /// Assert that events contain scale end
    fn assert_scale_end(&self) -> &Self;

    /// Assert that events do NOT contain any tap events
    fn assert_no_tap(&self) -> &Self;

    /// Assert that events do NOT contain any pan events
    fn assert_no_pan(&self) -> &Self;

    /// Assert event count
    fn assert_count(&self, count: usize) -> &Self;

    /// Get the first matching event
    fn first_of_type(&self, event_type: GestureEventType) -> Option<&GestureEvent>;
}

impl GestureEventAssertions for Vec<GestureEvent> {
    fn assert_tap(&self, count: u32) -> &Self {
        let found = self.iter().any(|e| {
            matches!(e.event_type, GestureEventType::Tap) && e.tap_count == count
        });
        assert!(found, "Expected tap with count={}, but found: {:?}", count, self);
        self
    }

    fn assert_long_press_start(&self) -> &Self {
        let found = self
            .iter()
            .any(|e| matches!(e.event_type, GestureEventType::LongPressStart));
        assert!(
            found,
            "Expected LongPressStart, but found: {:?}",
            self
        );
        self
    }

    fn assert_long_press_end(&self) -> &Self {
        let found = self
            .iter()
            .any(|e| matches!(e.event_type, GestureEventType::LongPressEnd));
        assert!(found, "Expected LongPressEnd, but found: {:?}", self);
        self
    }

    fn assert_pan_start(&self) -> &Self {
        let found = self
            .iter()
            .any(|e| matches!(e.event_type, GestureEventType::PanStart));
        assert!(found, "Expected PanStart, but found: {:?}", self);
        self
    }

    fn assert_pan_update(&self, expected_delta_x: f32, expected_delta_y: f32) -> &Self {
        let found = self.iter().any(|e| {
            if let GestureEventType::PanUpdate = e.event_type {
                (e.delta_x - expected_delta_x).abs() < 0.01
                    && (e.delta_y - expected_delta_y).abs() < 0.01
            } else {
                false
            }
        });
        assert!(
            found,
            "Expected PanUpdate with delta=({}, {}), but found: {:?}",
            expected_delta_x, expected_delta_y, self
        );
        self
    }

    fn assert_pan_end(&self) -> &Self {
        let found = self
            .iter()
            .any(|e| matches!(e.event_type, GestureEventType::PanEnd));
        assert!(found, "Expected PanEnd, but found: {:?}", self);
        self
    }

    fn assert_scale_start(&self) -> &Self {
        let found = self
            .iter()
            .any(|e| matches!(e.event_type, GestureEventType::ScaleStart));
        assert!(found, "Expected ScaleStart, but found: {:?}", self);
        self
    }

    fn assert_scale_update(&self, expected_scale: f32) -> &Self {
        let found = self.iter().any(|e| {
            if e.event_type == GestureEventType::ScaleUpdate {
                (e.scale - expected_scale).abs() < 0.01
            } else {
                false
            }
        });
        assert!(
            found,
            "Expected ScaleUpdate with scale={}, but found: {:?}",
            expected_scale, self
        );
        self
    }

    fn assert_scale_end(&self) -> &Self {
        let found = self
            .iter()
            .any(|e| matches!(e.event_type, GestureEventType::ScaleEnd));
        assert!(found, "Expected ScaleEnd, but found: {:?}", self);
        self
    }

    fn assert_no_tap(&self) -> &Self {
        let found = self
            .iter()
            .any(|e| matches!(e.event_type, GestureEventType::Tap));
        assert!(!found, "Expected NO tap events, but found: {:?}", self);
        self
    }

    fn assert_no_pan(&self) -> &Self {
        let found = self.iter().any(|e| {
            matches!(
                e.event_type,
                GestureEventType::PanStart | GestureEventType::PanUpdate | GestureEventType::PanEnd
            )
        });
        assert!(!found, "Expected NO pan events, but found: {:?}", self);
        self
    }

    fn assert_count(&self, count: usize) -> &Self {
        assert_eq!(
            self.len(),
            count,
            "Expected {} events, but found {}: {:?}",
            count,
            self.len(),
            self
        );
        self
    }

    fn first_of_type(&self, event_type: GestureEventType) -> Option<&GestureEvent> {
        self.iter().find(|e| e.event_type == event_type)
    }
}

/// Generate random pointer events for property testing
/// Note: This requires the `rand` crate to be added as a dev-dependency
pub struct RandomGestureGenerator {
    // Implementation would use rand crate
}

impl RandomGestureGenerator {
    /// Create a new random generator
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for RandomGestureGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pointer_event_builder() {
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 200.0);
        assert_eq!(down.event_type, PointerEventType::Down);
        assert_eq!(down.pointer_id, 0);
        assert_eq!(down.target_node_id, 1);
        assert_eq!(down.x, 100.0);
        assert_eq!(down.y, 200.0);

        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(150.0, 250.0);
        assert_eq!(move_evt.event_type, PointerEventType::Move);
        assert_eq!(move_evt.x, 150.0);
        assert_eq!(move_evt.y, 250.0);

        let up = PointerEventBuilder::new(0).node_id(1).up();
        assert_eq!(up.event_type, PointerEventType::Up);
    }

    #[test]
    fn test_event_assertions() {
        let events = vec![
            GestureEvent::tap(1, 0, 100.0, 100.0, 1, 0),
            GestureEvent::tap(1, 0, 100.0, 100.0, 2, 0),
        ];

        events.assert_tap(1).assert_tap(2).assert_count(2);
    }
}
