// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture System for WASM-side
//!
//! Provides a unified gesture API that maps to Host-side gesture recognition.
//!
//! ## Usage
//!
//! ### View-level syntax sugar:
//! ```ignore
//! View {
//!     onTap: |event| { /* event.tap_count */ }
//!     onDoubleTap: |event| { /* ... */ }
//!     onLongPress: |event| { /* event.phase: Began/Ended */ }
//!     onPan: |event| { /* event.phase: Began/Changed/Ended, event.delta_x/y */ }
//!     onScale: |event| { /* event.phase: Began/Changed/Ended, event.scale */ }
//!     onRotation: |event| { /* event.phase: Began/Changed/Ended, event.rotation */ }
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

/// Gesture phase - indicates the state of a continuous gesture
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GesturePhase {
    /// Gesture has started
    Began,
    /// Gesture is in progress (value has changed)
    Changed,
    /// Gesture has ended successfully
    Ended,
    /// Gesture was cancelled
    Cancelled,
}

/// Unified gesture event type
#[derive(Debug, Clone, Copy)]
pub enum GestureEventType {
    /// Tap detected (tap_count field indicates single/double/triple/etc)
    Tap,
    /// Long press gesture
    LongPress,
    /// Pan gesture (use phase to check state)
    Pan,
    /// Scale gesture (use phase to check state)
    Scale,
    /// Rotation gesture (use phase to check state)
    Rotation,
}

/// Unified gesture event
///
/// This is the argument passed to all gesture callbacks.
/// Different gesture types populate different fields.
///
/// For continuous gestures (Pan, Scale, Rotation), use `phase` to determine state:
/// ```rust
/// view.on_pan(|event| {
///     match event.phase {
///         GesturePhase::Began => println!("Pan started at {} {}", event.x, event.y),
///         GesturePhase::Changed => println!("Pan moved by {} {}", event.delta_x, event.delta_y),
///         GesturePhase::Ended => println!("Pan ended"),
///         GesturePhase::Cancelled => println!("Pan cancelled"),
///     }
/// });
/// ```
#[derive(Debug, Clone, Copy)]
pub struct GestureEvent {
    /// Type of gesture
    pub gesture_type: GestureEventType,
    /// Phase of the gesture (Began, Changed, Ended, Cancelled)
    pub phase: GesturePhase,
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
    /// For Scale: current scale factor (1.0 = original size)
    pub scale: f32,
    /// For Scale: scale delta since last update
    pub delta_scale: f32,
    /// For Rotation: current angle in radians
    pub rotation: f32,
    /// For Rotation: rotation delta since last update
    pub delta_rotation: f32,
    /// Timestamp (microseconds)
    pub timestamp_us: u64,
}

impl GestureEvent {
    /// Create a tap event with specified count
    pub fn tap(node_id: u32, x: f32, y: f32, tap_count: u32) -> Self {
        Self {
            gesture_type: GestureEventType::Tap,
            phase: GesturePhase::Ended, // Tap is discrete, so it's always Ended
            target_node_id: node_id,
            pointer_id: 0,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tap_count,
            scale: 1.0,
            delta_scale: 0.0,
            rotation: 0.0,
            delta_rotation: 0.0,
            timestamp_us: 0,
        }
    }

    /// Create a double tap event
    pub fn double_tap(node_id: u32, x: f32, y: f32) -> Self {
        Self::tap(node_id, x, y, 2)
    }

    /// Create a long press event
    pub fn long_press(node_id: u32, x: f32, y: f32, phase: GesturePhase) -> Self {
        Self {
            gesture_type: GestureEventType::LongPress,
            phase,
            target_node_id: node_id,
            pointer_id: 0,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tap_count: 0,
            scale: 1.0,
            delta_scale: 0.0,
            rotation: 0.0,
            delta_rotation: 0.0,
            timestamp_us: 0,
        }
    }

    /// Create a long press start event (deprecated, use long_press with Began)
    pub fn long_press_start(node_id: u32, x: f32, y: f32) -> Self {
        Self::long_press(node_id, x, y, GesturePhase::Began)
    }

    /// Create a pan event
    pub fn pan(node_id: u32, x: f32, y: f32, dx: f32, dy: f32, phase: GesturePhase) -> Self {
        Self {
            gesture_type: GestureEventType::Pan,
            phase,
            target_node_id: node_id,
            pointer_id: 0,
            x,
            y,
            delta_x: dx,
            delta_y: dy,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tap_count: 0,
            scale: 1.0,
            delta_scale: 0.0,
            rotation: 0.0,
            delta_rotation: 0.0,
            timestamp_us: 0,
        }
    }

    /// Create a pan start event (deprecated, use pan with Began)
    pub fn pan_start(node_id: u32, x: f32, y: f32) -> Self {
        Self::pan(node_id, x, y, 0.0, 0.0, GesturePhase::Began)
    }

    /// Create a pan update event (deprecated, use pan with Changed)
    pub fn pan_update(node_id: u32, x: f32, y: f32, dx: f32, dy: f32) -> Self {
        Self::pan(node_id, x, y, dx, dy, GesturePhase::Changed)
    }

    /// Create a pan end event (deprecated, use pan with Ended)
    pub fn pan_end(node_id: u32, x: f32, y: f32, vx: f32, vy: f32) -> Self {
        let mut event = Self::pan(node_id, x, y, 0.0, 0.0, GesturePhase::Ended);
        event.velocity_x = vx;
        event.velocity_y = vy;
        event
    }

    /// Create a scale event
    pub fn scale(node_id: u32, x: f32, y: f32, scale: f32, delta_scale: f32, phase: GesturePhase) -> Self {
        Self {
            gesture_type: GestureEventType::Scale,
            phase,
            target_node_id: node_id,
            pointer_id: 0,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tap_count: 0,
            scale,
            delta_scale,
            rotation: 0.0,
            delta_rotation: 0.0,
            timestamp_us: 0,
        }
    }

    /// Create a scale start event (deprecated, use scale with Began)
    pub fn scale_start(node_id: u32, x: f32, y: f32, scale: f32) -> Self {
        Self::scale(node_id, x, y, scale, 0.0, GesturePhase::Began)
    }

    /// Create a scale update event (deprecated, use scale with Changed)
    pub fn scale_update(node_id: u32, x: f32, y: f32, scale: f32, delta_scale: f32) -> Self {
        Self::scale(node_id, x, y, scale, delta_scale, GesturePhase::Changed)
    }

    /// Create a scale end event (deprecated, use scale with Ended)
    pub fn scale_end(node_id: u32, x: f32, y: f32) -> Self {
        Self::scale(node_id, x, y, 1.0, 0.0, GesturePhase::Ended)
    }

    /// Create a rotation event
    pub fn rotation(node_id: u32, x: f32, y: f32, angle: f32, delta_angle: f32, phase: GesturePhase) -> Self {
        Self {
            gesture_type: GestureEventType::Rotation,
            phase,
            target_node_id: node_id,
            pointer_id: 0,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            tap_count: 0,
            scale: 1.0,
            delta_scale: 0.0,
            rotation: angle,
            delta_rotation: delta_angle,
            timestamp_us: 0,
        }
    }

    /// Create a rotation start event (deprecated, use rotation with Began)
    pub fn rotation_start(node_id: u32, x: f32, y: f32, angle: f32) -> Self {
        Self::rotation(node_id, x, y, angle, 0.0, GesturePhase::Began)
    }

    /// Create a rotation update event (deprecated, use rotation with Changed)
    pub fn rotation_update(node_id: u32, x: f32, y: f32, angle: f32, delta_angle: f32) -> Self {
        Self::rotation(node_id, x, y, angle, delta_angle, GesturePhase::Changed)
    }

    /// Create a rotation end event (deprecated, use rotation with Ended)
    pub fn rotation_end(node_id: u32, x: f32, y: f32) -> Self {
        Self::rotation(node_id, x, y, 0.0, 0.0, GesturePhase::Ended)
    }

    /// Check if this is the beginning of a gesture
    pub fn is_began(&self) -> bool {
        self.phase == GesturePhase::Began
    }

    /// Check if this is a change/update to a gesture
    pub fn is_changed(&self) -> bool {
        self.phase == GesturePhase::Changed
    }

    /// Check if this is the end of a gesture
    pub fn is_ended(&self) -> bool {
        self.phase == GesturePhase::Ended
    }

    /// Check if this gesture was cancelled
    pub fn is_cancelled(&self) -> bool {
        self.phase == GesturePhase::Cancelled
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
    /// Create a tap gesture (defaults to single tap, use `.count(n)` to customize)
    pub fn new() -> Self {
        Self::single_tap()
    }

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

/// Scale gesture configuration (pinch-to-zoom)
pub struct ScaleGesture {
    pub min_scale_delta: f32,
    pub on_gesture_began: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_changed: Option<Box<dyn FnMut(GestureEvent)>>,
    pub on_gesture_ended: Option<Box<dyn FnMut(GestureEvent)>>,
}

impl ScaleGesture {
    pub fn new() -> Self {
        Self {
            min_scale_delta: 0.1,
            on_gesture_began: None,
            on_gesture_changed: None,
            on_gesture_ended: None,
        }
    }

    pub fn min_scale_delta(mut self, delta: f32) -> Self {
        self.min_scale_delta = delta;
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
}

impl Default for ScaleGesture {
    fn default() -> Self {
        Self::new()
    }
}

// Alias for backward compatibility
pub type PinchGesture = ScaleGesture;

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
    pub on_gesture_ended: Option<Box<dyn FnMut(GestureEvent)>>,
}

impl SequenceGesture {
    pub fn new(steps: Vec<GestureStep>) -> Self {
        Self {
            steps,
            on_gesture_judge_begin: None,
            on_gesture_ended: None,
        }
    }

    pub fn on_gesture_judge_begin(mut self, handler: impl FnMut(&GestureEvent) -> bool + 'static) -> Self {
        self.on_gesture_judge_begin = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_ended(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_ended = Some(Box::new(handler));
        self
    }
}

/// Parallel gesture - multiple gestures can be recognized simultaneously
///
/// DSL: `ParallelGesture([PinchGesture {}, RotationGesture {}])`
pub struct ParallelGesture {
    pub gestures: Vec<GestureStep>,
    pub on_gesture_judge_begin: Option<Box<dyn FnMut(&GestureEvent) -> bool>>,
    pub on_gesture_ended: Option<Box<dyn FnMut(GestureEvent)>>,
}

impl ParallelGesture {
    pub fn new(gestures: Vec<GestureStep>) -> Self {
        Self {
            gestures,
            on_gesture_judge_begin: None,
            on_gesture_ended: None,
        }
    }

    pub fn on_gesture_judge_begin(mut self, handler: impl FnMut(&GestureEvent) -> bool + 'static) -> Self {
        self.on_gesture_judge_begin = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_ended(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_ended = Some(Box::new(handler));
        self
    }
}

/// Exclusive gesture - only one gesture can win
///
/// DSL: `ExclusiveGesture([TapGesture {}, LongPressGesture {}])`
pub struct ExclusiveGesture {
    pub candidates: Vec<GestureStep>,
    pub on_gesture_judge_begin: Option<Box<dyn FnMut(&GestureEvent) -> bool>>,
    pub on_gesture_ended: Option<Box<dyn FnMut(GestureEvent)>>,
}

impl ExclusiveGesture {
    pub fn new(candidates: Vec<GestureStep>) -> Self {
        Self {
            candidates,
            on_gesture_judge_begin: None,
            on_gesture_ended: None,
        }
    }

    pub fn on_gesture_judge_begin(mut self, handler: impl FnMut(&GestureEvent) -> bool + 'static) -> Self {
        self.on_gesture_judge_begin = Some(Box::new(handler));
        self
    }

    pub fn on_gesture_ended(mut self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        self.on_gesture_ended = Some(Box::new(handler));
        self
    }
}

/// A single step in a composite gesture
pub enum GestureStep {
    Tap(TapGesture),
    LongPress(LongPressGesture),
    Pan(PanGesture),
    Scale(ScaleGesture),
    Rotation(RotationGesture),
    /// Nested exclusive gesture
    Exclusive(ExclusiveGesture),
    /// Nested simultaneous gesture
    Simultaneous(ParallelGesture),
    /// Nested sequenced gesture
    Sequenced(SequenceGesture),
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

impl From<ScaleGesture> for GestureStep {
    fn from(g: ScaleGesture) -> Self {
        GestureStep::Scale(g)
    }
}

impl From<RotationGesture> for GestureStep {
    fn from(g: RotationGesture) -> Self {
        GestureStep::Rotation(g)
    }
}

impl From<ExclusiveGesture> for GestureStep {
    fn from(g: ExclusiveGesture) -> Self {
        GestureStep::Exclusive(g)
    }
}

impl From<ParallelGesture> for GestureStep {
    fn from(g: ParallelGesture) -> Self {
        GestureStep::Simultaneous(g)
    }
}

impl From<SequenceGesture> for GestureStep {
    fn from(g: SequenceGesture) -> Self {
        GestureStep::Sequenced(g)
    }
}

// =============== Gesture Registration Helpers ===============

/// Gesture type bitflags for unified registration
pub const GESTURE_TAP: u16 = 1 << 0;
pub const GESTURE_LONG_PRESS: u16 = 1 << 1;
pub const GESTURE_PAN: u16 = 1 << 2;
pub const GESTURE_SCALE: u16 = 1 << 3;
pub const GESTURE_ROTATION: u16 = 1 << 4;

/// Config types for SetGestureConfig
pub const CONFIG_TAP_COUNT: u8 = 0;
pub const CONFIG_LONG_PRESS_TIMEOUT: u8 = 1;
pub const CONFIG_PAN_SLOP: u8 = 2;

/// Unified gesture registration - Phase 1 API
///
/// Example:
/// ```ignore
/// // Register tap and long press on same node
/// register_gesture(node_id, GESTURE_TAP | GESTURE_LONG_PRESS);
/// set_gesture_config(node_id, CONFIG_TAP_COUNT, 2); // double tap
/// ```
pub fn register_gesture(node_id: u32, mask: u16) {
    push_command!(SHARED_BUFFER, AttachClick, node_id);
    push_command!(SHARED_BUFFER, RegisterGesture, node_id, mask);
}

/// Set gesture configuration for a node
pub fn set_gesture_config(node_id: u32, config_type: u8, value: u32) {
    push_command!(SHARED_BUFFER, SetGestureConfig, node_id, config_type, value);
}

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
    // Unified tap handler with count=1 for single tap
    push_command!(SHARED_BUFFER, RegisterTapHandler, node_id, 1u32);

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
    // Use unified RegisterTapHandler with count=2 for double tap
    push_command!(SHARED_BUFFER, RegisterTapHandler, node_id, 2u32);

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
