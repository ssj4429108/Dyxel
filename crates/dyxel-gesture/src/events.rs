// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture Events
//! 
//! Defines the high-level gesture events that are dispatched to WASM.
//! These events are the output of the gesture recognition system.

/// Types of pointer events (raw input)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEventType {
    Down,
    Move,
    Up,
    Cancel,
}

/// Raw pointer event from input system
#[derive(Debug, Clone, Copy)]
pub struct PointerEvent {
    pub event_type: PointerEventType,
    pub pointer_id: u32,
    /// Timestamp in microseconds
    pub timestamp_us: u64,
    /// X position in logical pixels
    pub x: f32,
    /// Y position in logical pixels
    pub y: f32,
    /// Pressure (0.0 ~ 1.0)
    pub pressure: f32,
    /// Target node ID from hit test
    pub target_node_id: u32,
}

impl PointerEvent {
    /// Calculate squared distance to another pointer event
    pub fn squared_distance(&self, other: &PointerEvent) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }

    /// Calculate distance to another pointer event
    pub fn distance(&self, other: &PointerEvent) -> f32 {
        self.squared_distance(other).sqrt()
    }
}

/// Pointer tracking data for gesture recognizers
#[derive(Debug, Clone, Copy)]
pub struct PointerData {
    pub pointer_id: u32,
    pub start_x: f32,
    pub start_y: f32,
    pub current_x: f32,
    pub current_y: f32,
    pub start_time_us: u64,
    pub last_time_us: u64,
    pub pressure: f32,
}

impl PointerData {
    pub fn new(event: &PointerEvent) -> Self {
        Self {
            pointer_id: event.pointer_id,
            start_x: event.x,
            start_y: event.y,
            current_x: event.x,
            current_y: event.y,
            start_time_us: event.timestamp_us,
            last_time_us: event.timestamp_us,
            pressure: event.pressure,
        }
    }

    pub fn update(&mut self, event: &PointerEvent) {
        self.current_x = event.x;
        self.current_y = event.y;
        self.last_time_us = event.timestamp_us;
        self.pressure = event.pressure;
    }

    /// Total delta from start
    pub fn delta(&self) -> (f32, f32) {
        (self.current_x - self.start_x, self.current_y - self.start_y)
    }

    /// Total distance from start
    pub fn distance_from_start(&self) -> f32 {
        let (dx, dy) = self.delta();
        (dx * dx + dy * dy).sqrt()
    }

    /// Duration since start in microseconds
    pub fn duration_us(&self, current_time_us: u64) -> u64 {
        current_time_us.saturating_sub(self.start_time_us)
    }

    /// Duration since start in milliseconds
    pub fn duration_ms(&self, current_time_us: u64) -> u64 {
        self.duration_us(current_time_us) / 1000
    }
}

/// High-level gesture event types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureEventType {
    /// Tap detected (tap_count field indicates single/double/triple/etc)
    Tap,
    /// Long press started
    LongPressStart,
    /// Long press ended
    LongPressEnd,
    /// Pan started
    PanStart,
    /// Pan updated (finger moved)
    PanUpdate,
    /// Pan ended
    PanEnd,
    /// Scale started (pinch)
    ScaleStart,
    /// Scale updated
    ScaleUpdate,
    /// Scale ended
    ScaleEnd,
}

/// High-level gesture event
/// 
/// This is what gets dispatched to WASM. It contains all the
/// information needed to handle the gesture.
#[derive(Debug, Clone)]
pub struct GestureEvent {
    /// Type of gesture
    pub event_type: GestureEventType,
    /// Target node ID
    pub target_node_id: u32,
    /// Primary pointer ID
    pub primary_pointer_id: u32,
    /// Number of pointers involved
    pub pointer_count: u32,
    /// Current X position (or center for multi-touch)
    pub x: f32,
    /// Current Y position (or center for multi-touch)
    pub y: f32,
    /// For PanUpdate: delta X since last event
    pub delta_x: f32,
    /// For PanUpdate: delta Y since last event
    pub delta_y: f32,
    /// For ScaleUpdate: current scale factor
    pub scale: f32,
    /// For ScaleUpdate: scale delta since last event
    pub scale_delta: f32,
    /// For Tap: number of taps (1=single, 2=double, 3=triple, etc)
    pub tap_count: u32,
    /// Timestamp of the event
    pub timestamp_us: u64,
}

impl GestureEvent {
    /// Create a tap event with specified count
    pub fn tap(node_id: u32, pointer_id: u32, x: f32, y: f32, tap_count: u32, timestamp_us: u64) -> Self {
        Self {
            event_type: GestureEventType::Tap,
            target_node_id: node_id,
            primary_pointer_id: pointer_id,
            pointer_count: 1,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count,
            timestamp_us,
        }
    }

    /// Create a double tap event
    pub fn double_tap(node_id: u32, pointer_id: u32, x: f32, y: f32, timestamp_us: u64) -> Self {
        Self {
            event_type: GestureEventType::Tap,
            target_node_id: node_id,
            primary_pointer_id: pointer_id,
            pointer_count: 1,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: 2,
            timestamp_us,
        }
    }

    /// Create a long press start event
    pub fn long_press_start(node_id: u32, pointer_id: u32, x: f32, y: f32, timestamp_us: u64) -> Self {
        Self {
            event_type: GestureEventType::LongPressStart,
            target_node_id: node_id,
            primary_pointer_id: pointer_id,
            pointer_count: 1,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: 0,
            timestamp_us,
        }
    }

    /// Create a long press end event
    pub fn long_press_end(node_id: u32, pointer_id: u32, x: f32, y: f32, timestamp_us: u64) -> Self {
        Self {
            event_type: GestureEventType::LongPressEnd,
            target_node_id: node_id,
            primary_pointer_id: pointer_id,
            pointer_count: 1,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: 0,
            timestamp_us,
        }
    }

    /// Create a pan start event
    pub fn pan_start(node_id: u32, pointer_id: u32, x: f32, y: f32, timestamp_us: u64) -> Self {
        Self {
            event_type: GestureEventType::PanStart,
            target_node_id: node_id,
            primary_pointer_id: pointer_id,
            pointer_count: 1,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: 0,
            timestamp_us,
        }
    }

    /// Create a pan update event
    pub fn pan_update(
        node_id: u32,
        pointer_id: u32,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
        timestamp_us: u64,
    ) -> Self {
        Self {
            event_type: GestureEventType::PanUpdate,
            target_node_id: node_id,
            primary_pointer_id: pointer_id,
            pointer_count: 1,
            x,
            y,
            delta_x,
            delta_y,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: 0,
            timestamp_us,
        }
    }

    /// Create a pan end event
    pub fn pan_end(
        node_id: u32,
        pointer_id: u32,
        x: f32,
        y: f32,
        velocity_x: f32,
        velocity_y: f32,
        timestamp_us: u64,
    ) -> Self {
        Self {
            event_type: GestureEventType::PanEnd,
            target_node_id: node_id,
            primary_pointer_id: pointer_id,
            pointer_count: 1,
            x,
            y,
            delta_x: velocity_x,
            delta_y: velocity_y,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: 0,
            timestamp_us,
        }
    }
}

/// Gesture event dispatch result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DispatchResult {
    /// Event was handled
    Handled,
    /// Event was ignored
    Ignored,
    /// Event was cancelled
    Cancelled,
}
