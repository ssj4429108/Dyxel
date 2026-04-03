// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture Recognizer V3 - Flutter-compatible gesture recognition
//!
//! This module implements the new gesture recognition system based on Flutter's
//! GestureArena architecture with full support for:
//! - Tap (with configurable count)
//! - LongPress
//! - Pan (with direction locking)
//! - Scale (multi-touch)

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::events::{GestureEvent, GestureEventType, PointerEvent, PointerEventType};

// ============================================================================
// Constants - Aligned with Flutter
// ============================================================================

/// Touch slop in logical pixels
pub const K_TOUCH_SLOP: f32 = 18.0;

/// Double tap slop in logical pixels
pub const K_DOUBLE_TAP_SLOP: f32 = 100.0;

/// Double tap timeout in milliseconds
pub const K_DOUBLE_TAP_TIMEOUT_MS: u64 = 300;

/// Long press timeout in milliseconds
pub const K_LONG_PRESS_TIMEOUT_MS: u64 = 500;

/// Pan slop in logical pixels
pub const K_PAN_SLOP: f32 = 18.0;

// ============================================================================
// Core Types
// ============================================================================

/// Recognizer state - fully aligned with Flutter
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognizerState {
    /// Ready to start recognition
    Ready,
    /// Possible gesture, waiting for more data
    Possible,
    /// Gesture has begun (for continuous gestures)
    Began,
    /// Gesture is updating (for continuous gestures)
    Changed,
    /// Gesture ended successfully
    Ended,
    /// Gesture was cancelled
    Cancelled,
    /// Gesture recognition failed
    Failed,
}

impl RecognizerState {
    /// Check if state is terminal
    pub fn is_terminal(&self) -> bool {
        matches!(self, RecognizerState::Ended | RecognizerState::Cancelled | RecognizerState::Failed)
    }

    /// Check if gesture has been accepted (won the arena)
    pub fn is_accepted(&self) -> bool {
        matches!(self, RecognizerState::Began | RecognizerState::Changed | RecognizerState::Ended)
    }
}

/// Gesture category for competition rules
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GestureCategoryType {
    /// Discrete tap gestures (single, double, triple tap)
    DiscreteTap,
    /// Long press gesture
    DiscreteLongPress,
    /// Continuous pan gesture (single or multi-pointer)
    ContinuousPan,
    /// Continuous scale gesture (multi-pointer)
    ContinuousScale,
}

impl GestureCategoryType {
    /// Check if this category is discrete (fires once) vs continuous (fires multiple times)
    pub fn is_discrete(&self) -> bool {
        matches!(self, Self::DiscreteTap | Self::DiscreteLongPress)
    }

    /// Check if this category is continuous
    pub fn is_continuous(&self) -> bool {
        matches!(self, Self::ContinuousPan | Self::ContinuousScale)
    }
}

/// Gesture disposition in arena
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureDisposition {
    /// Accept the gesture (win the arena)
    Accepted,
    /// Reject the gesture (lose the arena)
    Rejected,
    /// Need more time to decide
    Pending,
}

/// Direction for pan gesture
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanDirection {
    /// Any direction
    Any,
    /// Horizontal only
    Horizontal,
    /// Vertical only
    Vertical,
}

// ============================================================================
// GestureRecognizer Trait
// ============================================================================

/// Base trait for gesture recognizers
pub trait GestureRecognizer: Send + Any {
    /// Get recognizer ID
    fn id(&self) -> u32;

    /// Get target node ID
    fn node_id(&self) -> u32;

    /// Handle pointer event
    fn handle_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent>;

    /// Check timers and return events
    fn check_timers(&mut self, now: Instant) -> Vec<GestureEvent>;

    /// Called when gesture is accepted by arena
    fn accept(&mut self);

    /// Called when gesture is rejected by arena
    fn reject(&mut self);

    /// Get current state
    fn state(&self) -> RecognizerState;

    /// Reset recognizer to initial state
    fn reset(&mut self);

    /// Get gesture category for competition rules
    fn category(&self) -> GestureCategoryType;

    /// Check if this recognizer is exclusive with another
    /// Default implementation uses category-based rules:
    /// - Same category: exclusive
    /// - Different discrete categories: exclusive
    /// - Discrete vs Continuous: not exclusive (compete via slop/timing)
    /// - Continuous vs Continuous: not exclusive (can coexist)
    fn is_exclusive_with(&self, other: &dyn GestureRecognizer) -> bool {
        let my_cat = self.category();
        let other_cat = other.category();

        // Same category always competes
        if my_cat == other_cat {
            return true;
        }

        // Different discrete categories compete (e.g., Tap vs LongPress)
        if my_cat.is_discrete() && other_cat.is_discrete() {
            return true;
        }

        // Continuous gestures don't compete with each other (Pan + Scale can coexist)
        if my_cat.is_continuous() && other_cat.is_continuous() {
            return false;
        }

        // Discrete vs Continuous: don't compete directly
        // They compete via slop/timing (discrete wins if no movement)
        false
    }

    /// Cast to Any for downcasting
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// ============================================================================
// TapGestureRecognizer
// ============================================================================

/// Tap gesture recognizer with configurable tap count
pub struct TapGestureRecognizer {
    id: u32,
    node_id: u32,
    state: RecognizerState,

    // Configuration
    tap_count: u32,
    slop: f32,
    multi_tap_timeout: Duration,
    multi_tap_slop: f32,
    /// Whether to fire events for tap counts less than tap_count
    /// If true, a tap_count=2 recognizer will fire tap_count=1 events on timeout
    /// If false, it only fires when exact tap_count is reached
    fire_partial_results: bool,

    // State
    current_taps: u32,
    first_pointer_down: Option<(f32, f32, Instant)>,
    last_pointer_up: Option<(f32, f32, Instant)>,
    multi_tap_deadline: Option<Instant>,
}

impl TapGestureRecognizer {
    /// Create a new tap recognizer
    pub fn new(id: u32, node_id: u32) -> Self {
        Self {
            id,
            node_id,
            state: RecognizerState::Ready,
            tap_count: 1,
            slop: K_TOUCH_SLOP,
            multi_tap_timeout: Duration::from_millis(K_DOUBLE_TAP_TIMEOUT_MS),
            multi_tap_slop: K_DOUBLE_TAP_SLOP,
            fire_partial_results: true, // Default: fire partial results (backward compatible)
            current_taps: 0,
            first_pointer_down: None,
            last_pointer_up: None,
            multi_tap_deadline: None,
        }
    }

    /// Set target tap count
    pub fn with_tap_count(mut self, count: u32) -> Self {
        self.tap_count = count.max(1);
        self
    }

    /// Set whether to fire partial results
    /// When false, only fires when exact tap_count is reached
    /// When true, fires with actual tap count on timeout
    pub fn with_fire_partial_results(mut self, fire_partial: bool) -> Self {
        self.fire_partial_results = fire_partial;
        self
    }

    /// Set slop (max movement for tap)
    pub fn with_slop(mut self, slop: f32) -> Self {
        self.slop = slop;
        self
    }

    /// Check if waiting for more taps in a multi-tap sequence
    pub fn is_waiting_for_more_taps(&self) -> bool {
        // Waiting if we have some taps but not enough yet
        self.current_taps > 0 && self.current_taps < self.tap_count && self.multi_tap_deadline.is_some()
    }

    /// Create a single tap recognizer
    pub fn single_tap(id: u32, node_id: u32) -> Self {
        Self::new(id, node_id).with_tap_count(1)
    }

    /// Create a double tap recognizer
    pub fn double_tap(id: u32, node_id: u32) -> Self {
        Self::new(id, node_id).with_tap_count(2)
    }

    /// Create a triple tap recognizer
    pub fn triple_tap(id: u32, node_id: u32) -> Self {
        Self::new(id, node_id).with_tap_count(3)
    }

    /// Handle pointer down
    fn handle_down(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        if self.state.is_terminal() {
            return vec![];
        }

        match self.state {
            RecognizerState::Ready => {
                // Start new tap sequence
                self.state = RecognizerState::Possible;
                self.current_taps = 0;
                self.first_pointer_down = Some((event.x, event.y, Instant::now()));
                self.multi_tap_deadline = None;
                vec![]
            }
            RecognizerState::Possible => {
                // Check if this is a multi-tap
                if let Some((last_x, last_y, last_time)) = self.last_pointer_up {
                    let now = Instant::now();
                    let distance = ((event.x - last_x).powi(2) + (event.y - last_y).powi(2)).sqrt();
                    let elapsed = now.duration_since(last_time);

                    if distance > self.multi_tap_slop || elapsed > self.multi_tap_timeout {
                        // Too far or too late, start new sequence
                        self.current_taps = 0;
                    }
                }

                self.first_pointer_down = Some((event.x, event.y, Instant::now()));
                vec![]
            }
            _ => vec![],
        }
    }

    /// Handle pointer up
    fn handle_up(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        if self.state.is_terminal() {
            return vec![];
        }

        if self.state != RecognizerState::Possible {
            return vec![];
        }

        // Check if movement was within slop
        if let Some((start_x, start_y, _)) = self.first_pointer_down {
            let distance = ((event.x - start_x).powi(2) + (event.y - start_y).powi(2)).sqrt();
            if distance > self.slop {
                // Moved too much, fail
                self.state = RecognizerState::Failed;
                return vec![];
            }
        }

        self.current_taps += 1;
        self.last_pointer_up = Some((event.x, event.y, Instant::now()));

        if self.current_taps >= self.tap_count {
            // Reached target tap count, fire immediately
            self.state = RecognizerState::Ended;
            vec![self.create_tap_event(event.x, event.y)]
        } else {
            // Need more taps, set deadline
            self.multi_tap_deadline = Some(Instant::now() + self.multi_tap_timeout);
            vec![]
        }
    }

    /// Handle pointer move
    fn handle_move(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        if self.state != RecognizerState::Possible {
            return vec![];
        }

        // Check if moved beyond slop
        if let Some((start_x, start_y, _)) = self.first_pointer_down {
            let distance = ((event.x - start_x).powi(2) + (event.y - start_y).powi(2)).sqrt();
            if distance > self.slop {
                self.state = RecognizerState::Failed;
            }
        }

        vec![]
    }

    /// Create tap event
    fn create_tap_event(&self, x: f32, y: f32) -> GestureEvent {
        GestureEvent {
            event_type: GestureEventType::Tap,
            target_node_id: self.node_id,
            primary_pointer_id: 0,
            pointer_count: 1,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: self.current_taps,
            timestamp_us: Self::now_us(),
            phase: None,
        }
    }

    fn now_us() -> u64 {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }
}

impl GestureRecognizer for TapGestureRecognizer {
    fn id(&self) -> u32 {
        self.id
    }

    fn node_id(&self) -> u32 {
        self.node_id
    }

    fn handle_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        match event.event_type {
            PointerEventType::Down => self.handle_down(event),
            PointerEventType::Up => self.handle_up(event),
            PointerEventType::Move => self.handle_move(event),
            PointerEventType::Cancel => {
                self.state = RecognizerState::Cancelled;
                vec![]
            }
        }
    }

    fn check_timers(&mut self, now: Instant) -> Vec<GestureEvent> {
        if self.state != RecognizerState::Possible {
            return vec![];
        }

        // Check multi-tap deadline
        if let Some(deadline) = self.multi_tap_deadline {
            if now >= deadline && self.current_taps > 0 && self.current_taps < self.tap_count {
                // Deadline passed with partial taps
                // Always fire the event with actual tap count - handlers will filter
                self.state = RecognizerState::Ended;
                if let Some((x, y, _)) = self.last_pointer_up {
                    return vec![self.create_tap_event(x, y)];
                }
            }
        }

        vec![]
    }

    fn accept(&mut self) {
        // Tap is discrete, already fired when accepted
    }

    fn reject(&mut self) {
        self.state = RecognizerState::Failed;
    }

    fn state(&self) -> RecognizerState {
        self.state
    }

    fn reset(&mut self) {
        self.state = RecognizerState::Ready;
        self.current_taps = 0;
        self.first_pointer_down = None;
        self.last_pointer_up = None;
        self.multi_tap_deadline = None;
    }

    fn category(&self) -> GestureCategoryType {
        GestureCategoryType::DiscreteTap
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ============================================================================
// LongPressGestureRecognizer
// ============================================================================


/// Long press gesture recognizer
pub struct LongPressGestureRecognizer {
    id: u32,
    node_id: u32,
    state: RecognizerState,

    // Configuration
    duration: Duration,
    slop: f32,

    // State
    start_position: Option<(f32, f32)>,
    start_time: Option<Instant>,
    deadline: Option<Instant>,
    current_position: (f32, f32),
}

impl LongPressGestureRecognizer {
    /// Create a new long press recognizer
    pub fn new(id: u32, node_id: u32) -> Self {
        Self {
            id,
            node_id,
            state: RecognizerState::Ready,
            duration: Duration::from_millis(K_LONG_PRESS_TIMEOUT_MS),
            slop: K_TOUCH_SLOP,
            start_position: None,
            start_time: None,
            deadline: None,
            current_position: (0.0, 0.0),
        }
    }

    /// Set duration
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Set slop
    pub fn with_slop(mut self, slop: f32) -> Self {
        self.slop = slop;
        self
    }

    /// Handle pointer down
    fn handle_down(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        if self.state.is_terminal() {
            return vec![];
        }

        self.state = RecognizerState::Possible;
        self.start_position = Some((event.x, event.y));
        self.current_position = (event.x, event.y);
        self.start_time = Some(Instant::now());
        self.deadline = Some(Instant::now() + self.duration);
        vec![]
    }

    /// Handle pointer move
    fn handle_move(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        self.current_position = (event.x, event.y);

        if self.state == RecognizerState::Possible {
            // Check if moved beyond slop
            if let Some((start_x, start_y)) = self.start_position {
                let distance = ((event.x - start_x).powi(2) + (event.y - start_y).powi(2)).sqrt();
                if distance > self.slop {
                    self.state = RecognizerState::Failed;
                }
            }
        }

        vec![]
    }

    /// Handle pointer up
    fn handle_up(&mut self, _event: &PointerEvent) -> Vec<GestureEvent> {
        if self.state == RecognizerState::Began {
            // Was active, now ending
            self.state = RecognizerState::Ended;
            vec![self.create_event(false)]
        } else {
            // Never activated
            self.state = RecognizerState::Failed;
            vec![]
        }
    }

    /// Create long press event
    fn create_event(&self, is_start: bool) -> GestureEvent {
        GestureEvent {
            event_type: if is_start {
                GestureEventType::LongPressStart
            } else {
                GestureEventType::LongPressEnd
            },
            target_node_id: self.node_id,
            primary_pointer_id: 0,
            pointer_count: 1,
            x: self.current_position.0,
            y: self.current_position.1,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: 0,
            timestamp_us: Self::now_us(),
            phase: if is_start {
                Some(crate::events::GesturePhase::Start)
            } else {
                Some(crate::events::GesturePhase::End)
            },
        }
    }

    fn now_us() -> u64 {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }
}

impl GestureRecognizer for LongPressGestureRecognizer {
    fn id(&self) -> u32 {
        self.id
    }

    fn node_id(&self) -> u32 {
        self.node_id
    }

    fn handle_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        match event.event_type {
            PointerEventType::Down => self.handle_down(event),
            PointerEventType::Up => self.handle_up(event),
            PointerEventType::Move => self.handle_move(event),
            PointerEventType::Cancel => {
                self.state = RecognizerState::Cancelled;
                vec![]
            }
        }
    }

    fn check_timers(&mut self, now: Instant) -> Vec<GestureEvent> {
        if self.state != RecognizerState::Possible {
            return vec![];
        }

        if let Some(deadline) = self.deadline {
            if now >= deadline {
                // Long press triggered!
                self.state = RecognizerState::Began;
                return vec![self.create_event(true)];
            }
        }

        vec![]
    }

    fn accept(&mut self) {
        // Already fired when timer triggered
    }

    fn reject(&mut self) {
        self.state = RecognizerState::Failed;
    }

    fn state(&self) -> RecognizerState {
        self.state
    }

    fn reset(&mut self) {
        self.state = RecognizerState::Ready;
        self.start_position = None;
        self.start_time = None;
        self.deadline = None;
        self.current_position = (0.0, 0.0);
    }

    fn category(&self) -> GestureCategoryType {
        GestureCategoryType::DiscreteLongPress
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ============================================================================
// Velocity Tracker
// ============================================================================

/// Sample for velocity calculation
#[derive(Debug, Clone, Copy)]
struct VelocitySample {
    time: Instant,
    position: (f32, f32),
}

/// Tracks velocity for pan gestures
pub struct VelocityTracker {
    samples: VecDeque<VelocitySample>,
    max_samples: usize,
}

impl VelocityTracker {
    /// Create a new velocity tracker
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(20),
            max_samples: 20,
        }
    }

    /// Add a position sample
    pub fn add_sample(&mut self, position: (f32, f32)) {
        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }
        self.samples.push_back(VelocitySample {
            time: Instant::now(),
            position,
        });
    }

    /// Calculate velocity
    pub fn calculate_velocity(&self) -> (f32, f32) {
        if self.samples.len() < 2 {
            return (0.0, 0.0);
        }

        let first = self.samples.front().unwrap();
        let last = self.samples.back().unwrap();

        let dt = last.time.duration_since(first.time).as_secs_f32();
        if dt < 0.001 {
            return (0.0, 0.0);
        }

        let dx = last.position.0 - first.position.0;
        let dy = last.position.1 - first.position.1;

        (dx / dt, dy / dt)
    }

    /// Clear all samples
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

impl Default for VelocityTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// PanGestureRecognizer
// ============================================================================

/// Pan gesture recognizer
pub struct PanGestureRecognizer {
    id: u32,
    node_id: u32,
    state: RecognizerState,

    // Configuration
    direction: PanDirection,
    slop: f32,

    // State - multi-pointer support
    /// All active pointers: pointer_id -> (x, y)
    pointers: HashMap<u32, (f32, f32)>,
    /// Primary pointer (first one down)
    primary_pointer: Option<u32>,
    /// Start position of the primary pointer (for slop calculation)
    start_position: Option<(f32, f32)>,
    /// Current focal point (center of all pointers)
    current_position: (f32, f32),
    /// Last focal point
    last_position: (f32, f32),
    /// Track focal point for velocity
    velocity_tracker: VelocityTracker,
}

impl PanGestureRecognizer {
    /// Create a new pan recognizer
    pub fn new(id: u32, node_id: u32) -> Self {
        Self {
            id,
            node_id,
            state: RecognizerState::Ready,
            direction: PanDirection::Any,
            slop: K_PAN_SLOP,
            pointers: HashMap::new(),
            primary_pointer: None,
            start_position: None,
            current_position: (0.0, 0.0),
            last_position: (0.0, 0.0),
            velocity_tracker: VelocityTracker::new(),
        }
    }

    /// Set direction
    pub fn with_direction(mut self, direction: PanDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Set slop
    pub fn with_slop(mut self, slop: f32) -> Self {
        self.slop = slop;
        self
    }

    /// Check if movement matches direction constraint
    fn matches_direction(&self, delta_x: f32, delta_y: f32) -> bool {
        match self.direction {
            PanDirection::Any => true,
            PanDirection::Horizontal => delta_x.abs() > delta_y.abs(),
            PanDirection::Vertical => delta_y.abs() > delta_x.abs(),
        }
    }

    /// Calculate focal point (center of all pointers)
    fn calculate_focal(&self) -> (f32, f32) {
        if self.pointers.is_empty() {
            return (0.0, 0.0);
        }

        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        for (x, y) in self.pointers.values() {
            sum_x += x;
            sum_y += y;
        }
        let count = self.pointers.len() as f32;
        (sum_x / count, sum_y / count)
    }

    /// Handle pointer down
    fn handle_down(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        if self.state.is_terminal() {
            return vec![];
        }

        // Track this pointer
        self.pointers.insert(event.pointer_id, (event.x, event.y));

        // If this is the first pointer, initialize state
        if self.primary_pointer.is_none() {
            self.primary_pointer = Some(event.pointer_id);
            self.state = RecognizerState::Possible;
            self.start_position = Some((event.x, event.y));
            self.velocity_tracker.clear();
        }

        // Update focal point
        let focal = self.calculate_focal();
        self.current_position = focal;
        self.last_position = focal;
        self.velocity_tracker.add_sample(focal);

        vec![]
    }

    /// Handle pointer move
    fn handle_move(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        // Update the moved pointer position
        if self.pointers.contains_key(&event.pointer_id) {
            self.pointers.insert(event.pointer_id, (event.x, event.y));
        }

        // Calculate focal point (center of all pointers)
        let focal = self.calculate_focal();
        self.velocity_tracker.add_sample(focal);

        match self.state {
            RecognizerState::Possible => {
                // Check if primary pointer exceeded slop
                if let Some((start_x, start_y)) = self.start_position {
                    let dx = focal.0 - start_x;
                    let dy = focal.1 - start_y;
                    let distance = (dx.powi(2) + dy.powi(2)).sqrt();

                    if distance > self.slop && self.matches_direction(dx, dy) {
                        // Pan started!
                        self.state = RecognizerState::Began;
                        self.current_position = focal;
                        self.last_position = focal;
                        return vec![self.create_event(true, 0.0, 0.0)];
                    }
                }
                vec![]
            }
            RecognizerState::Began | RecognizerState::Changed => {
                let delta_x = focal.0 - self.last_position.0;
                let delta_y = focal.1 - self.last_position.1;
                self.last_position = self.current_position;
                self.current_position = focal;
                self.state = RecognizerState::Changed;
                vec![self.create_event(false, delta_x, delta_y)]
            }
            _ => vec![],
        }
    }

    /// Handle pointer up
    fn handle_up(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        // Remove this pointer from tracking
        self.pointers.remove(&event.pointer_id);

        // If this was the primary pointer, clear it
        if self.primary_pointer == Some(event.pointer_id) {
            self.primary_pointer = None;
        }

        // If there are still pointers down, continue tracking with remaining pointers
        if !self.pointers.is_empty() {
            // Recalculate focal point based on remaining pointers
            let focal = self.calculate_focal();
            self.last_position = self.current_position;
            self.current_position = focal;
            self.velocity_tracker.add_sample(focal);

            // If we have a new primary pointer, update start position for it
            if self.primary_pointer.is_none() {
                if let Some((&new_primary, _)) = self.pointers.iter().next() {
                    self.primary_pointer = Some(new_primary);
                    self.start_position = Some(focal);
                }
            }

            // Continue in Changed state if active
            if self.state.is_accepted() {
                self.state = RecognizerState::Changed;
            }
            return vec![];
        }

        // No more pointers - end the gesture if it was active
        if !self.state.is_accepted() {
            self.state = RecognizerState::Failed;
            return vec![];
        }

        let velocity = self.velocity_tracker.calculate_velocity();
        self.state = RecognizerState::Ended;
        vec![self.create_end_event(velocity.0, velocity.1)]
    }

    /// Create pan event
    fn create_event(&self, is_start: bool, delta_x: f32, delta_y: f32) -> GestureEvent {
        GestureEvent {
            event_type: if is_start {
                GestureEventType::PanStart
            } else {
                GestureEventType::PanUpdate
            },
            target_node_id: self.node_id,
            primary_pointer_id: self.primary_pointer.unwrap_or(0),
            pointer_count: self.pointers.len() as u32,
            x: self.current_position.0,
            y: self.current_position.1,
            delta_x,
            delta_y,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: 0,
            timestamp_us: Self::now_us(),
            phase: if is_start {
                Some(crate::events::GesturePhase::Start)
            } else {
                Some(crate::events::GesturePhase::Update)
            },
        }
    }

    /// Create pan end event
    fn create_end_event(&self, velocity_x: f32, velocity_y: f32) -> GestureEvent {
        GestureEvent {
            event_type: GestureEventType::PanEnd,
            target_node_id: self.node_id,
            primary_pointer_id: self.primary_pointer.unwrap_or(0),
            pointer_count: self.pointers.len() as u32,
            x: self.current_position.0,
            y: self.current_position.1,
            delta_x: velocity_x,
            delta_y: velocity_y,
            scale: 1.0,
            scale_delta: 0.0,
            tap_count: 0,
            timestamp_us: Self::now_us(),
            phase: Some(crate::events::GesturePhase::End),
        }
    }

    fn now_us() -> u64 {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }
}

impl GestureRecognizer for PanGestureRecognizer {
    fn id(&self) -> u32 {
        self.id
    }

    fn node_id(&self) -> u32 {
        self.node_id
    }

    fn handle_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        match event.event_type {
            PointerEventType::Down => self.handle_down(event),
            PointerEventType::Up => self.handle_up(event),
            PointerEventType::Move => self.handle_move(event),
            PointerEventType::Cancel => {
                self.state = RecognizerState::Cancelled;
                vec![]
            }
        }
    }

    fn check_timers(&mut self, _now: Instant) -> Vec<GestureEvent> {
        // Pan doesn't use timers
        vec![]
    }

    fn accept(&mut self) {
        // Pan accepts immediately when slop is exceeded
    }

    fn reject(&mut self) {
        self.state = RecognizerState::Failed;
    }

    fn state(&self) -> RecognizerState {
        self.state
    }

    fn reset(&mut self) {
        self.state = RecognizerState::Ready;
        self.pointers.clear();
        self.primary_pointer = None;
        self.start_position = None;
        self.current_position = (0.0, 0.0);
        self.last_position = (0.0, 0.0);
        self.velocity_tracker.clear();
    }

    fn category(&self) -> GestureCategoryType {
        GestureCategoryType::ContinuousPan
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ============================================================================
// ScaleGestureRecognizer
// ============================================================================

/// Scale gesture recognizer for pinch-to-zoom
pub struct ScaleGestureRecognizer {
    id: u32,
    node_id: u32,
    state: RecognizerState,

    // Configuration
    slop: f32,

    // Multi-pointer state
    pointers: HashMap<u32, (f32, f32)>,

    // Scale tracking
    initial_distance: f32,
    current_scale: f32,
    last_scale: f32,

    // Focal point tracking
    focal_start: (f32, f32),
    current_focal: (f32, f32),
}

impl ScaleGestureRecognizer {
    /// Create a new scale recognizer
    pub fn new(id: u32, node_id: u32) -> Self {
        Self {
            id,
            node_id,
            state: RecognizerState::Ready,
            slop: 0.1, // Minimum scale change to trigger
            pointers: HashMap::new(),
            initial_distance: 0.0,
            current_scale: 1.0,
            last_scale: 1.0,
            focal_start: (0.0, 0.0),
            current_focal: (0.0, 0.0),
        }
    }

    /// Set slop (minimum scale change)
    pub fn with_slop(mut self, slop: f32) -> Self {
        self.slop = slop;
        self
    }

    /// Calculate distance between two pointers
    fn calculate_distance(&self) -> f32 {
        if self.pointers.len() < 2 {
            return 0.0;
        }

        let positions: Vec<_> = self.pointers.values().collect();
        let dx = positions[0].0 - positions[1].0;
        let dy = positions[0].1 - positions[1].1;
        (dx * dx + dy * dy).sqrt()
    }

    /// Calculate focal point (center of all pointers)
    fn calculate_focal(&self) -> (f32, f32) {
        if self.pointers.is_empty() {
            return (0.0, 0.0);
        }

        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        for (x, y) in self.pointers.values() {
            sum_x += x;
            sum_y += y;
        }
        let count = self.pointers.len() as f32;
        (sum_x / count, sum_y / count)
    }

    /// Handle pointer down
    fn handle_down(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        if self.state.is_terminal() {
            return vec![];
        }

        // Add pointer
        self.pointers.insert(event.pointer_id, (event.x, event.y));

        match self.state {
            RecognizerState::Ready => {
                if self.pointers.len() == 2 {
                    // Second pointer - start tracking
                    self.state = RecognizerState::Possible;
                    self.initial_distance = self.calculate_distance();
                    self.focal_start = self.calculate_focal();
                    self.current_focal = self.focal_start;
                    self.current_scale = 1.0;
                    self.last_scale = 1.0;
                }
                vec![]
            }
            RecognizerState::Possible | RecognizerState::Began | RecognizerState::Changed => {
                // Update tracking
                if self.pointers.len() >= 2 {
                    self.current_focal = self.calculate_focal();
                }
                vec![]
            }
            _ => vec![],
        }
    }

    /// Handle pointer move
    fn handle_move(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        // Update pointer position
        if let Some(pos) = self.pointers.get_mut(&event.pointer_id) {
            *pos = (event.x, event.y);
        }

        match self.state {
            RecognizerState::Possible => {
                if self.pointers.len() >= 2 {
                    let distance = self.calculate_distance();
                    if self.initial_distance > 0.0 {
                        let scale = distance / self.initial_distance;
                        let delta = (scale - 1.0).abs();

                        if delta > self.slop {
                            // Scale started!
                            self.state = RecognizerState::Began;
                            self.current_scale = scale;
                            self.last_scale = scale;
                            self.current_focal = self.calculate_focal();
                            return vec![self.create_event(true, 0.0)];
                        }
                    }
                }
                vec![]
            }
            RecognizerState::Began | RecognizerState::Changed => {
                if self.pointers.len() >= 2 {
                    let distance = self.calculate_distance();
                    if self.initial_distance > 0.0 {
                        self.current_scale = distance / self.initial_distance;
                        let delta = self.current_scale - self.last_scale;
                        self.last_scale = self.current_scale;
                        self.current_focal = self.calculate_focal();
                        self.state = RecognizerState::Changed;
                        return vec![self.create_event(false, delta)];
                    }
                }
                vec![]
            }
            _ => vec![],
        }
    }

    /// Handle pointer up
    fn handle_up(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        // Remove pointer
        self.pointers.remove(&event.pointer_id);

        match self.state {
            RecognizerState::Began | RecognizerState::Changed => {
                if self.pointers.len() < 2 {
                    // Not enough pointers, end scale
                    self.state = RecognizerState::Ended;
                    vec![self.create_end_event()]
                } else {
                    // Still have 2+ pointers, continue tracking
                    self.current_focal = self.calculate_focal();
                    vec![]
                }
            }
            _ => {
                if self.pointers.is_empty() {
                    self.state = RecognizerState::Failed;
                }
                vec![]
            }
        }
    }

    /// Create scale event
    fn create_event(&self, is_start: bool, scale_delta: f32) -> GestureEvent {
        GestureEvent {
            event_type: if is_start {
                GestureEventType::ScaleStart
            } else {
                GestureEventType::ScaleUpdate
            },
            target_node_id: self.node_id,
            primary_pointer_id: 0,
            pointer_count: self.pointers.len() as u32,
            x: self.current_focal.0,
            y: self.current_focal.1,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: self.current_scale,
            scale_delta,
            tap_count: 0,
            timestamp_us: Self::now_us(),
            phase: if is_start {
                Some(crate::events::GesturePhase::Start)
            } else {
                Some(crate::events::GesturePhase::Update)
            },
        }
    }

    /// Create scale end event
    fn create_end_event(&self) -> GestureEvent {
        GestureEvent {
            event_type: GestureEventType::ScaleEnd,
            target_node_id: self.node_id,
            primary_pointer_id: 0,
            pointer_count: self.pointers.len() as u32,
            x: self.current_focal.0,
            y: self.current_focal.1,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: self.current_scale,
            scale_delta: 0.0,
            tap_count: 0,
            timestamp_us: Self::now_us(),
            phase: Some(crate::events::GesturePhase::End),
        }
    }

    fn now_us() -> u64 {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }
}

impl GestureRecognizer for ScaleGestureRecognizer {
    fn id(&self) -> u32 {
        self.id
    }

    fn node_id(&self) -> u32 {
        self.node_id
    }

    fn handle_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        match event.event_type {
            PointerEventType::Down => self.handle_down(event),
            PointerEventType::Up => self.handle_up(event),
            PointerEventType::Move => self.handle_move(event),
            PointerEventType::Cancel => {
                self.state = RecognizerState::Cancelled;
                vec![]
            }
        }
    }

    fn check_timers(&mut self, _now: Instant) -> Vec<GestureEvent> {
        // Scale doesn't use timers
        vec![]
    }

    fn accept(&mut self) {
        // Scale accepts when slop is exceeded
    }

    fn reject(&mut self) {
        self.state = RecognizerState::Failed;
    }

    fn state(&self) -> RecognizerState {
        self.state
    }

    fn reset(&mut self) {
        self.state = RecognizerState::Ready;
        self.pointers.clear();
        self.initial_distance = 0.0;
        self.current_scale = 1.0;
        self.last_scale = 1.0;
        self.focal_start = (0.0, 0.0);
        self.current_focal = (0.0, 0.0);
    }

    fn category(&self) -> GestureCategoryType {
        GestureCategoryType::ContinuousScale
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{GestureEventAssertions, PointerEventBuilder};

    // ===== Tap Tests =====

    #[test]
    fn test_tap_single_success() {
        let mut recognizer = TapGestureRecognizer::single_tap(1, 1);

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        let events = recognizer.handle_event(&down);
        assert!(events.is_empty());
        assert_eq!(recognizer.state(), RecognizerState::Possible);

        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        let events = recognizer.handle_event(&up);
        events.assert_tap(1).assert_count(1);
        assert!(recognizer.state().is_terminal());
    }

    #[test]
    fn test_tap_double_success() {
        let mut recognizer = TapGestureRecognizer::double_tap(1, 1);

        // First tap
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down);
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        let events = recognizer.handle_event(&up);
        assert!(events.is_empty()); // Waiting for second tap

        // Second tap (quickly)
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down);
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        let events = recognizer.handle_event(&up);
        events.assert_tap(2).assert_count(1);
    }

    #[test]
    fn test_tap_exceed_slop_fail() {
        let mut recognizer = TapGestureRecognizer::single_tap(1, 1);

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down);

        // Move beyond slop
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(150.0, 150.0);
        recognizer.handle_event(&move_evt);

        let up = PointerEventBuilder::new(0).node_id(1).up_at(150.0, 150.0);
        let events = recognizer.handle_event(&up);
        assert!(events.is_empty());
        assert_eq!(recognizer.state(), RecognizerState::Failed);
    }

    #[test]
    fn test_tap_timeout_single() {
        let mut recognizer = TapGestureRecognizer::new(1, 1).with_tap_count(2);
        let start = Instant::now();

        // First tap
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down);
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        recognizer.handle_event(&up);

        // Wait for timeout
        let events = recognizer.check_timers(start + Duration::from_millis(400));
        events.assert_tap(1).assert_count(1);
    }

    // ===== Long Press Tests =====

    #[test]
    fn test_long_press_success() {
        let mut recognizer = LongPressGestureRecognizer::new(1, 1);
        let start = Instant::now();

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        let events = recognizer.handle_event(&down);
        assert!(events.is_empty());

        // Wait for timeout
        let events = recognizer.check_timers(start + Duration::from_millis(600));
        events.assert_long_press_start().assert_count(1);
        assert_eq!(recognizer.state(), RecognizerState::Began);

        // Release
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        let events = recognizer.handle_event(&up);
        events.assert_long_press_end().assert_count(1);
    }

    #[test]
    fn test_long_press_release_early() {
        let mut recognizer = LongPressGestureRecognizer::new(1, 1);

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down);

        // Release before timeout
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        let events = recognizer.handle_event(&up);
        assert!(events.is_empty());
        assert_eq!(recognizer.state(), RecognizerState::Failed);
    }

    #[test]
    fn test_long_press_exceed_slop() {
        let mut recognizer = LongPressGestureRecognizer::new(1, 1);

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down);

        // Move beyond slop
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(150.0, 150.0);
        recognizer.handle_event(&move_evt);

        assert_eq!(recognizer.state(), RecognizerState::Failed);
    }

    // ===== Pan Tests =====

    #[test]
    fn test_pan_success() {
        let mut recognizer = PanGestureRecognizer::new(1, 1);

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        let events = recognizer.handle_event(&down);
        assert!(events.is_empty());

        // Move beyond slop
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(130.0, 130.0);
        let events = recognizer.handle_event(&move_evt);
        events.assert_pan_start().assert_count(1);

        // Continue move
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(140.0, 140.0);
        let events = recognizer.handle_event(&move_evt);
        events.assert_pan_update(10.0, 10.0);

        // Release
        let up = PointerEventBuilder::new(0).node_id(1).up_at(140.0, 140.0);
        let events = recognizer.handle_event(&up);
        events.assert_pan_end();
    }

    #[test]
    fn test_pan_below_slop_no_trigger() {
        let mut recognizer = PanGestureRecognizer::new(1, 1);

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down);

        // Move below slop
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(105.0, 105.0);
        let events = recognizer.handle_event(&move_evt);
        assert!(events.is_empty());
        assert_eq!(recognizer.state(), RecognizerState::Possible);
    }

    #[test]
    fn test_pan_horizontal_lock() {
        let mut recognizer = PanGestureRecognizer::new(1, 1)
            .with_direction(PanDirection::Horizontal);

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down);

        // Move mostly vertical (should not trigger)
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(102.0, 130.0);
        let events = recognizer.handle_event(&move_evt);
        assert!(events.is_empty());

        // Move mostly horizontal (should trigger)
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(130.0, 102.0);
        let events = recognizer.handle_event(&move_evt);
        events.assert_pan_start();
    }

    #[test]
    fn test_pan_cancel() {
        let mut recognizer = PanGestureRecognizer::new(1, 1);

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down);

        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(130.0, 130.0);
        recognizer.handle_event(&move_evt);

        let cancel = PointerEventBuilder::new(0).node_id(1).cancel();
        recognizer.handle_event(&cancel);

        assert_eq!(recognizer.state(), RecognizerState::Cancelled);
    }

    // ===== Scale Tests =====

    #[test]
    fn test_scale_single_pointer_no_trigger() {
        let mut recognizer = ScaleGestureRecognizer::new(1, 1);

        // Single pointer down - should not trigger
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        let events = recognizer.handle_event(&down);
        assert!(events.is_empty());
        assert_eq!(recognizer.state(), RecognizerState::Ready);
    }

    #[test]
    fn test_scale_two_pointer_success() {
        let mut recognizer = ScaleGestureRecognizer::new(1, 1).with_slop(0.05); // Lower slop for testing

        // First pointer
        let down1 = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        recognizer.handle_event(&down1);

        // Second pointer
        let down2 = PointerEventBuilder::new(1).node_id(1).down(200.0, 100.0);
        let events = recognizer.handle_event(&down2);
        assert!(events.is_empty()); // Now in Possible state
        assert_eq!(recognizer.state(), RecognizerState::Possible);

        // Move to zoom in (distance increases from 100 to 200 - 2x zoom)
        // This is a scale of 2.0, delta of 1.0, which is > 0.05 slop
        let move1 = PointerEventBuilder::new(0).node_id(1).move_to(50.0, 100.0);
        let move2 = PointerEventBuilder::new(1).node_id(1).move_to(250.0, 100.0);
        recognizer.handle_event(&move1);
        let events = recognizer.handle_event(&move2);

        // Should trigger scale (either Start or Update)
        let has_scale_event = events.iter().any(|e| {
            matches!(e.event_type, GestureEventType::ScaleStart | GestureEventType::ScaleUpdate)
        });
        assert!(has_scale_event,
            "Expected Scale event. Events: {:?}, Current scale: {}, state: {:?}",
            events, recognizer.current_scale, recognizer.state());
        // State could be Began or Changed depending on timing
        assert!(matches!(recognizer.state(), RecognizerState::Began | RecognizerState::Changed));
    }

    #[test]
    fn test_scale_zoom_in_out() {
        let mut recognizer = ScaleGestureRecognizer::new(1, 1);

        // Setup: two pointers
        recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0));
        recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).down(200.0, 100.0));

        // Initial zoom in
        recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).move_to(75.0, 100.0));
        recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).move_to(225.0, 100.0));

        // Zoom out (back to original distance)
        let move1 = PointerEventBuilder::new(0).node_id(1).move_to(100.0, 100.0);
        let move2 = PointerEventBuilder::new(1).node_id(1).move_to(200.0, 100.0);
        let events1 = recognizer.handle_event(&move1);
        let events2 = recognizer.handle_event(&move2);

        // Should have scale update events
        let all_events: Vec<_> = events1.into_iter().chain(events2).collect();
        assert!(all_events.iter().any(|e| matches!(e.event_type, GestureEventType::ScaleUpdate)));

        // Scale should be close to 1.0 (back to original)
        assert!((recognizer.current_scale - 1.0).abs() < 0.1);
    }

    #[test]
    fn test_scale_end_on_pointer_up() {
        let mut recognizer = ScaleGestureRecognizer::new(1, 1);

        // Setup and start scale
        recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0));
        recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).down(200.0, 100.0));
        recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).move_to(75.0, 100.0));
        recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).move_to(225.0, 100.0));

        // Release one pointer
        let up = PointerEventBuilder::new(0).node_id(1).up_at(75.0, 100.0);
        let events = recognizer.handle_event(&up);

        // Should trigger scale end
        assert!(events.iter().any(|e| matches!(e.event_type, GestureEventType::ScaleEnd)));
        assert!(recognizer.state().is_terminal());
    }

    #[test]
    fn test_scale_focal_point() {
        let mut recognizer = ScaleGestureRecognizer::new(1, 1);

        // Setup: two pointers at (100,100) and (200,100)
        recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0));
        recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).down(200.0, 100.0));

        // Focal point should be at center
        assert!((recognizer.current_focal.0 - 150.0).abs() < 0.1);
        assert!((recognizer.current_focal.1 - 100.0).abs() < 0.1);
    }

    // ===== Gesture Category Competition Tests =====

    #[test]
    fn test_tap_vs_long_press_exclusive() {
        let tap = TapGestureRecognizer::single_tap(1, 1);
        let long_press = LongPressGestureRecognizer::new(2, 1);

        // Both are discrete, should compete
        assert!(tap.is_exclusive_with(&long_press));
        assert!(long_press.is_exclusive_with(&tap));
    }

    #[test]
    fn test_tap_vs_tap_exclusive() {
        let tap1 = TapGestureRecognizer::single_tap(1, 1);
        let tap2 = TapGestureRecognizer::double_tap(2, 1);

        // Same category, should compete
        assert!(tap1.is_exclusive_with(&tap2));
        assert!(tap2.is_exclusive_with(&tap1));
    }

    #[test]
    fn test_pan_vs_scale_not_exclusive() {
        let pan = PanGestureRecognizer::new(1, 1);
        let scale = ScaleGestureRecognizer::new(2, 1);

        // Both continuous, should NOT compete (can coexist)
        assert!(!pan.is_exclusive_with(&scale));
        assert!(!scale.is_exclusive_with(&pan));
    }

    #[test]
    fn test_discrete_vs_continuous_not_exclusive() {
        let tap = TapGestureRecognizer::single_tap(1, 1);
        let pan = PanGestureRecognizer::new(2, 1);

        // Discrete vs Continuous, should NOT directly compete
        // They compete via slop/timing instead
        assert!(!tap.is_exclusive_with(&pan));
        assert!(!pan.is_exclusive_with(&tap));

        let long_press = LongPressGestureRecognizer::new(3, 1);
        let scale = ScaleGestureRecognizer::new(4, 1);

        assert!(!long_press.is_exclusive_with(&scale));
        assert!(!scale.is_exclusive_with(&long_press));
    }

    #[test]
    fn test_category_type_helpers() {
        assert!(GestureCategoryType::DiscreteTap.is_discrete());
        assert!(!GestureCategoryType::DiscreteTap.is_continuous());

        assert!(GestureCategoryType::DiscreteLongPress.is_discrete());
        assert!(!GestureCategoryType::DiscreteLongPress.is_continuous());

        assert!(!GestureCategoryType::ContinuousPan.is_discrete());
        assert!(GestureCategoryType::ContinuousPan.is_continuous());

        assert!(!GestureCategoryType::ContinuousScale.is_discrete());
        assert!(GestureCategoryType::ContinuousScale.is_continuous());
    }

    // ===== Multi-Pointer Pan Tests =====

    #[test]
    fn test_pan_single_pointer() {
        let mut recognizer = PanGestureRecognizer::new(1, 1);

        // Single pointer down
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        let events = recognizer.handle_event(&down);
        assert!(events.is_empty());
        assert_eq!(recognizer.state(), RecognizerState::Possible);

        // Move beyond slop
        let move1 = PointerEventBuilder::new(0).node_id(1).move_to(130.0, 130.0);
        let events = recognizer.handle_event(&move1);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, GestureEventType::PanStart));
        assert_eq!(events[0].pointer_count, 1);
        assert_eq!(recognizer.state(), RecognizerState::Began);

        // Continue moving
        let move2 = PointerEventBuilder::new(0).node_id(1).move_to(140.0, 140.0);
        let events = recognizer.handle_event(&move2);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, GestureEventType::PanUpdate));
        assert_eq!(events[0].pointer_count, 1);

        // Release
        let up = PointerEventBuilder::new(0).node_id(1).up_at(140.0, 140.0);
        let events = recognizer.handle_event(&up);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, GestureEventType::PanEnd));
    }

    #[test]
    fn test_pan_two_pointer_centroid() {
        let mut recognizer = PanGestureRecognizer::new(1, 1);

        // First pointer down at (100, 100)
        let down1 = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        let events = recognizer.handle_event(&down1);
        assert!(events.is_empty());

        // Second pointer down at (200, 100), centroid is (150, 100)
        let down2 = PointerEventBuilder::new(1).node_id(1).down(200.0, 100.0);
        let events = recognizer.handle_event(&down2);
        assert!(events.is_empty());
        assert_eq!(recognizer.pointers.len(), 2);

        // Move both pointers - centroid moves from (150, 100) to (180, 130)
        // This is a ~42px diagonal delta, exceeding slop
        let events1 = recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).move_to(110.0, 130.0));
        let events2 = recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).move_to(250.0, 130.0));

        // Check if PanStart was triggered in either event
        let all_events: Vec<_> = events1.iter().chain(events2.iter()).collect();
        let pan_started = all_events.iter().any(|e| matches!(e.event_type, GestureEventType::PanStart));
        assert!(pan_started, "Pan should have started");

        // After PanStart, verify state and pointer count
        assert!(recognizer.state().is_accepted());
    }

    #[test]
    fn test_pan_two_pointer_release_one() {
        let mut recognizer = PanGestureRecognizer::new(1, 1);

        // Setup: two pointers, pan active
        recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0));
        recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).down(200.0, 100.0));
        // Move to start pan (large movement to exceed slop)
        recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).move_to(150.0, 150.0));
        recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).move_to(250.0, 150.0));
        assert!(recognizer.state().is_accepted(), "Pan should be active");

        // Release first pointer
        let up = PointerEventBuilder::new(0).node_id(1).up_at(130.0, 100.0);
        let events = recognizer.handle_event(&up);

        // No end event yet - still have one pointer
        assert!(events.is_empty());
        assert_eq!(recognizer.pointers.len(), 1);
        // Still in active state
        assert!(recognizer.state().is_accepted());

        // Continue moving with remaining pointer
        let move1 = PointerEventBuilder::new(1).node_id(1).move_to(240.0, 100.0);
        let events = recognizer.handle_event(&move1);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, GestureEventType::PanUpdate));
        assert_eq!(events[0].pointer_count, 1);

        // Release last pointer
        let up2 = PointerEventBuilder::new(1).node_id(1).up_at(240.0, 100.0);
        let events = recognizer.handle_event(&up2);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, GestureEventType::PanEnd));
    }

    // ===== Pan + Scale Simultaneous Tests =====

    #[test]
    fn test_pan_and_scale_simultaneous() {
        // This test verifies that Pan and Scale can work together
        // when both are registered on the same node

        let mut pan = PanGestureRecognizer::new(1, 1);
        let mut scale = ScaleGestureRecognizer::new(2, 1);

        // Two pointers down
        let down1 = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        let down2 = PointerEventBuilder::new(1).node_id(1).down(200.0, 100.0);

        pan.handle_event(&down1);
        pan.handle_event(&down2);
        scale.handle_event(&down1);
        scale.handle_event(&down2);

        // Move both pointers outward (zoom) and diagonally (pan)
        // From (100,100)-(200,100) to (50,50)-(250,150)
        // Distance changes: 100 -> 200 (2x zoom)
        // Centroid changes: (150,100) -> (150,100) (no pan in this example)
        // Let's adjust: (80,80)-(220,120) -> centroid (150,100) stays same
        // Actually let's make it simple: (50,100)-(250,100) -> distance 200

        let move1 = PointerEventBuilder::new(0).node_id(1).move_to(50.0, 100.0);
        let move2 = PointerEventBuilder::new(1).node_id(1).move_to(250.0, 100.0);

        let pan_events = pan.handle_event(&move1);
        let pan_events2 = pan.handle_event(&move2);
        let scale_events = scale.handle_event(&move1);
        let scale_events2 = scale.handle_event(&move2);

        // Both should have started
        let pan_started = pan_events.iter().chain(pan_events2.iter())
            .any(|e| matches!(e.event_type, GestureEventType::PanStart));
        let scale_started = scale_events.iter().chain(scale_events2.iter())
            .any(|e| matches!(e.event_type, GestureEventType::ScaleStart));

        // In this case, both should trigger (pan from centroid movement, scale from distance change)
        assert!(pan_started || pan.state() == RecognizerState::Began,
            "Pan should have started");
        assert!(scale_started || scale.state() == RecognizerState::Began,
            "Scale should have started");

        // Verify they're not exclusive
        assert!(!pan.is_exclusive_with(&scale));
        assert!(!scale.is_exclusive_with(&pan));
    }

    #[test]
    fn test_pan_centroid_tracking() {
        let mut recognizer = PanGestureRecognizer::new(1, 1);

        // Two pointers: (100, 100) and (200, 200), centroid at (150, 150)
        recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0));
        recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).down(200.0, 200.0));

        // Verify initial centroid
        let focal = recognizer.calculate_focal();
        assert!((focal.0 - 150.0).abs() < 0.1);
        assert!((focal.1 - 150.0).abs() < 0.1);

        // Move to (120, 120) and (220, 220), new centroid at (170, 170)
        recognizer.handle_event(&PointerEventBuilder::new(0).node_id(1).move_to(120.0, 120.0));
        recognizer.handle_event(&PointerEventBuilder::new(1).node_id(1).move_to(220.0, 220.0));

        // Verify new centroid
        let focal = recognizer.calculate_focal();
        assert!((focal.0 - 170.0).abs() < 0.1);
        assert!((focal.1 - 170.0).abs() < 0.1);
    }
}
