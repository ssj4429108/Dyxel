// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture System for WASM-side
//!
//! Provides a unified gesture API that maps to Host-side gesture recognition.
//!
//! ## Usage
//!
//! ### View-level syntax sugar (old API, updated signature):
//! ```ignore
//! View {
//!     onTap: |event| { /* ... */ }
//!     onDoubleTap: |event| { /* ... */ }
//!     onLongPress: |event| { /* ... */ }
//!     onPanUpdate: |event| { /* ... */ }
//! }
//! ```
//!
//! ### Explicit gesture configuration:
//! ```ignore
//! View {
//!     gesture: TapGesture {
//!         count: 2,
//!         onGestureEnded: |event| { /* ... */ }
//!     }
//! }
//! ```
//!
//! ### Composite gestures:
//! ```ignore
//! View {
//!     gesture: SequenceGesture([
//!         TapGesture { count: 2, onGestureEnded: |e| {} },
//!         LongPressGesture { onGestureEnded: |e| {} }
//!     ])
//! }
//! ```

use crate::{push_command, SHARED_BUFFER};

/// Unified gesture event type
#[derive(Debug, Clone, Copy)]
pub enum GestureEventType {
    /// Tap detected (tap_count field indicates single/double/triple/etc)
    Tap,
    /// Long press began
    LongPressStart,
    /// Long press ended
    LongPressEnd,
    /// Pan began
    PanStart,
    /// Pan updated (finger moved)
    PanUpdate,
    /// Pan ended
    PanEnd,
}

/// Unified gesture event
///
/// This is the argument passed to all gesture callbacks.
/// Different gesture types populate different fields.
#[derive(Debug, Clone, Copy)]
pub struct GestureEvent {
    /// Type of gesture
    pub gesture_type: GestureEventType,
    /// Target node ID
    pub target_node_id: u32,
    /// Pointer ID
    pub pointer_id: u32,
    /// X position (logical pixels)
    pub x: f32,
    /// Y position (logical pixels)
    pub y: f32,
    /// Delta X (for PanUpdate)
    pub delta_x: f32,
    /// Delta Y (for PanUpdate)
    pub delta_y: f32,
    /// Velocity X (for PanEnd)
    pub velocity_x: f32,
    /// Velocity Y (for PanEnd)
    pub velocity_y: f32,
    /// For Tap: number of taps (1=single, 2=double, 3=triple, etc)
    pub tap_count: u32,
    /// Timestamp (microseconds)
    pub timestamp_us: u64,
}

impl GestureEvent {
    /// Create a tap event with specified count
    pub fn tap(node_id: u32, x: f32, y: f32, tap_count: u32) -> Self {
        Self {
            gesture_type: GestureEventType::Tap,
            target_node_id: node_id,
            pointer_id: 0,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tap_count,
            timestamp_us: 0,
        }
    }

    /// Create a double tap event
    pub fn double_tap(node_id: u32, x: f32, y: f32) -> Self {
        Self::tap(node_id, x, y, 2)
    }

    /// Create a long press start event
    pub fn long_press_start(node_id: u32, x: f32, y: f32) -> Self {
        Self {
            gesture_type: GestureEventType::LongPressStart,
            target_node_id: node_id,
            pointer_id: 0,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tap_count: 0,
            timestamp_us: 0,
        }
    }

    /// Create a pan start event
    pub fn pan_start(node_id: u32, x: f32, y: f32) -> Self {
        Self {
            gesture_type: GestureEventType::PanStart,
            target_node_id: node_id,
            pointer_id: 0,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tap_count: 0,
            timestamp_us: 0,
        }
    }

    /// Create a pan update event
    pub fn pan_update(node_id: u32, x: f32, y: f32, dx: f32, dy: f32) -> Self {
        Self {
            gesture_type: GestureEventType::PanUpdate,
            target_node_id: node_id,
            pointer_id: 0,
            x,
            y,
            delta_x: dx,
            delta_y: dy,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tap_count: 0,
            timestamp_us: 0,
        }
    }
}

/// Direction for pan/swipe gestures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanDirection {
    Any,
    Horizontal,
    Vertical,
    Left,
    Right,
    Up,
    Down,
}

/// Tap gesture configuration
///
/// DSL: `TapGesture { count: 2, max_duration_ms: 300 }`
pub struct TapGesture {
    pub count: u32,
    pub max_duration_ms: u64,
    pub on_gesture_began: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_ended: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_cancelled: Option<Box<dyn FnMut(GestureEvent)>>,
}

impl TapGesture {
    /// Create a single tap gesture
    pub fn single_tap() -> Self {
        Self {
            count: 1,
            max_duration_ms: 300,
            on_gesture_began: None,
            on_gesture_ended: None,
            on_gesture_cancelled: None,
        }
    }

    /// Create a double tap gesture
    pub fn double_tap() -> Self {
        Self {
            count: 2,
            max_duration_ms: 300,
            on_gesture_began: None,
            on_gesture_ended: None,
            on_gesture_cancelled: None,
        }
    }

    pub fn count(mut self, count: u32) -> Self {
        self.count = count;
        self
    }

    pub fn max_duration_ms(mut self, ms: u64) -> Self {
        self.max_duration_ms = ms;
        self
    }

    pub fn on_gesture_began(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_began = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_ended(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_ended = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_cancelled(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_cancelled = Some(Box::new(handler));
        self
    }
}

impl Default for TapGesture {
    fn default() -> Self {
        Self::single_tap()
    }
}

/// Long press gesture configuration
///
/// DSL: `LongPressGesture { duration_ms: 500 }`
pub struct LongPressGesture {
    pub duration_ms: u64,
    pub on_gesture_began: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_ended: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_cancelled: Option<Box<dyn FnMut(GestureEvent)>>,
}

impl LongPressGesture {
    pub fn new() -> Self {
        Self {
            duration_ms: 500,
            on_gesture_began: None,
            on_gesture_ended: None,
            on_gesture_cancelled: None,
        }
    }

    pub fn duration_ms(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    pub fn on_gesture_began(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_began = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_ended(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_ended = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_cancelled(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_cancelled = Some(Box::new(handler));
        self
    }
}

impl Default for LongPressGesture {
    fn default() -> Self {
        Self::new()
    }
}

/// Pan gesture configuration
///
/// DSL: `PanGesture { direction: Horizontal, min_distance: 20.0 }`
pub struct PanGesture {
    pub direction: PanDirection,
    pub min_distance: f32,
    pub on_gesture_began: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_changed: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_ended: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_cancelled: Option<Box<dyn FnMut(GestureEvent)>>,
}

impl PanGesture {
    pub fn new() -> Self {
        Self {
            direction: PanDirection::Any,
            min_distance: 18.0,
            on_gesture_began: None,
            on_gesture_changed: None,
            on_gesture_ended: None,
            on_gesture_cancelled: None,
        }
    }

    pub fn direction(mut self, dir: PanDirection) -> Self {
        self.direction = dir;
        self
    }

    pub fn min_distance(mut self, dist: f32) -> Self {
        self.min_distance = dist;
        self
    }

    pub fn on_gesture_began(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_began = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_changed(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_changed = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_ended(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_ended = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_cancelled(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_cancelled = Some(Box::new(handler));
        self
    }
}

impl Default for PanGesture {
    fn default() -> Self {
        Self::new()
    }
}

/// Pinch gesture configuration (scale)
pub struct PinchGesture {
    pub min_scale: f32,
    pub on_gesture_began: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_changed: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_ended: Option<Box<dyn FnMut(GestureEvent)>>,
}

impl PinchGesture {
    pub fn new() -> Self {
        Self {
            min_scale: 0.1,
            on_gesture_began: None,
            on_gesture_changed: None,
            on_gesture_ended: None,
        }
    }

    pub fn min_scale(mut self, scale: f32) -> Self {
        self.min_scale = scale;
        self
    }
}

impl Default for PinchGesture {
    fn default() -> Self {
        Self::new()
    }
}

/// Rotation gesture configuration
pub struct RotationGesture {
    pub min_angle: f32,
    pub on_gesture_began: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_changed: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_ended: Option<Box<dyn FnMut(GestureEvent)>>,
}

impl RotationGesture {
    pub fn new() -> Self {
        Self {
            min_angle: 0.1,
            on_gesture_began: None,
            on_gesture_changed: None,
            on_gesture_ended: None,
        }
    }
}

impl Default for RotationGesture {
    fn default() -> Self {
        Self::new()
    }
}

// =============== Composite Gestures ===============

/// Sequence gesture - gestures must complete in order
///
/// DSL: `SequenceGesture([TapGesture { count: 2 }, LongPressGesture {}])`
pub struct SequenceGesture {
    pub steps: Vec<GestureStep>,
    pub on_gesture_judge_begin: Option<Box<dyn FnMut(&GestureEvent) -> bool>>,
}

impl SequenceGesture {
    pub fn new(steps: Vec<GestureStep>) -> Self {
        Self {
            steps,
            on_gesture_judge_begin: None,
        }
    }

    pub fn on_gesture_judge_begin(mut self, handler: impl FnMut(&GestureEvent) -> bool + 'static) -> Self {
        self.on_gesture_judge_begin = Some(Box::new(handler));
        self
    }
}

/// Parallel gesture - multiple gestures can be recognized simultaneously
///
/// DSL: `ParallelGesture([PinchGesture {}, RotationGesture {}])`
pub struct ParallelGesture {
    pub gestures: Vec<GestureStep>,
    pub on_gesture_judge_begin: Option<Box<dyn FnMut(&GestureEvent) -> bool>>,
}

impl ParallelGesture {
    pub fn new(gestures: Vec<GestureStep>) -> Self {
        Self {
            gestures,
            on_gesture_judge_begin: None,
        }
    }

    pub fn on_gesture_judge_begin(mut self, handler: impl FnMut(&GestureEvent) -> bool + 'static) -> Self {
        self.on_gesture_judge_begin = Some(Box::new(handler));
        self
    }
}

/// Exclusive gesture - only one gesture can win
///
/// DSL: `ExclusiveGesture([TapGesture {}, LongPressGesture {}])`
pub struct ExclusiveGesture {
    pub candidates: Vec<GestureStep>,
    pub on_gesture_judge_begin: Option<Box<dyn FnMut(&GestureEvent) -> bool>>,
}

impl ExclusiveGesture {
    pub fn new(candidates: Vec<GestureStep>) -> Self {
        Self {
            candidates,
            on_gesture_judge_begin: None,
        }
    }

    pub fn on_gesture_judge_begin(mut self, handler: impl FnMut(&GestureEvent) -> bool + 'static) -> Self {
        self.on_gesture_judge_begin = Some(Box::new(handler));
        self
    }
}

/// A single step in a composite gesture
pub enum GestureStep {
    Tap(TapGesture),
    LongPress(LongPressGesture),
    Pan(PanGesture),
    Pinch(PinchGesture),
    Rotation(RotationGesture),
}

// Helper constructors for GestureStep
impl From<TapGesture> for GestureStep {
    fn from(g: TapGesture) -> Self {
        GestureStep::Tap(g)
    }
}

impl From<LongPressGesture> for GestureStep {
    fn from(g: LongPressGesture) -> Self {
        GestureStep::LongPress(g)
    }
}

impl From<PanGesture> for GestureStep {
    fn from(g: PanGesture) -> Self {
        GestureStep::Pan(g)
    }
}

impl From<PinchGesture> for GestureStep {
    fn from(g: PinchGesture) -> Self {
        GestureStep::Pinch(g)
    }
}

impl From<RotationGesture> for GestureStep {
    fn from(g: RotationGesture) -> Self {
        GestureStep::Rotation(g)
    }
}

// =============== Gesture Registration Helpers ===============

/// Register a tap handler for a node (new API with GestureEvent)
pub fn register_tap_handler<F>(node_id: u32, mut handler: F)
where
    F: FnMut(GestureEvent) + 'static,
{
    use std::cell::RefCell;
    use std::collections::HashMap;
    
    thread_local! {
        static TAP_HANDLERS_V2: RefCell<HashMap<u32, Box<dyn FnMut(GestureEvent)>>> = RefCell::new(HashMap::new());
    }
    
    push_command!(SHARED_BUFFER, AttachClick, node_id);
    push_command!(SHARED_BUFFER, RegisterTapHandler, node_id);
    
    TAP_HANDLERS_V2.with(|h| {
        h.borrow_mut().insert(node_id, Box::new(move |event| handler(event)));
    });
}

/// Register a double tap handler for a node (new API with GestureEvent)
pub fn register_double_tap_handler<F>(node_id: u32, mut handler: F)
where
    F: FnMut(GestureEvent) + 'static,
{
    use std::cell::RefCell;
    use std::collections::HashMap;
    
    thread_local! {
        static DOUBLE_TAP_HANDLERS_V2: RefCell<HashMap<u32, Box<dyn FnMut(GestureEvent)>>> = RefCell::new(HashMap::new());
    }
    
    push_command!(SHARED_BUFFER, AttachClick, node_id);
    push_command!(SHARED_BUFFER, RegisterDoubleTapHandler, node_id);
    
    DOUBLE_TAP_HANDLERS_V2.with(|h| {
        h.borrow_mut().insert(node_id, Box::new(move |event| handler(event)));
    });
}

/// Register a long press handler for a node (new API with GestureEvent)
pub fn register_long_press_handler<F>(node_id: u32, mut handler: F)
where
    F: FnMut(GestureEvent) + 'static,
{
    use std::cell::RefCell;
    use std::collections::HashMap;
    
    thread_local! {
        static LONG_PRESS_HANDLERS_V2: RefCell<HashMap<u32, Box<dyn FnMut(GestureEvent)>>> = RefCell::new(HashMap::new());
    }
    
    push_command!(SHARED_BUFFER, AttachClick, node_id);
    push_command!(SHARED_BUFFER, RegisterLongPressHandler, node_id);
    
    LONG_PRESS_HANDLERS_V2.with(|h| {
        h.borrow_mut().insert(node_id, Box::new(move |event| handler(event)));
    });
}

/// Register a pan handler for a node (new API with GestureEvent)
pub fn register_pan_handler<F>(node_id: u32, mut handler: F)
where
    F: FnMut(GestureEvent) + 'static,
{
    use std::cell::RefCell;
    use std::collections::HashMap;
    
    thread_local! {
        static PAN_HANDLERS_V2: RefCell<HashMap<u32, Box<dyn FnMut(GestureEvent)>>> = RefCell::new(HashMap::new());
    }
    
    push_command!(SHARED_BUFFER, AttachClick, node_id);
    push_command!(SHARED_BUFFER, RegisterPanHandler, node_id);
    
    PAN_HANDLERS_V2.with(|h| {
        h.borrow_mut().insert(node_id, Box::new(move |event| handler(event)));
    });
}
