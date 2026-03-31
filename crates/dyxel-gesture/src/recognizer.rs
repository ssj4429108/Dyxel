// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture Recognizers
//! 
//! Implementations of various gesture recognizers:
//! - TapGestureRecognizer: Single, double, triple tap
//! - LongPressGestureRecognizer: Press and hold
//! - PanGestureRecognizer: Drag gestures

use crate::events::{GestureEvent, PointerEvent, PointerEventType, PointerData};
use crate::GestureSettings;

/// State of a gesture recognizer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognizerState {
    /// Ready to recognize gesture
    Possible,
    /// Gesture is being tracked but not yet accepted
    Began,
    /// Gesture has been accepted by the arena
    Accepted,
    /// Gesture has been rejected by the arena
    Rejected,
    /// Gesture has been recognized and is active
    Changed,
    /// Gesture has ended
    Ended,
    /// Gesture was cancelled
    Cancelled,
}

/// Configuration for gesture recognizers
#[derive(Debug, Clone, Copy)]
pub struct GestureConfig {
    pub settings: GestureSettings,
    /// Target node ID
    pub target_node_id: u32,
}

/// Base trait for all gesture recognizers
/// 
/// Inspired by UIKit's UIGestureRecognizer and Flutter's GestureRecognizer
pub trait GestureRecognizer: Send {
    /// Handle a pointer event
    /// 
    /// Returns any gesture events that should be dispatched
    fn handle_event(
        &mut self,
        event: &PointerEvent,
        tracked_pointers: &std::collections::HashMap<u32, PointerData>,
    ) -> Vec<GestureEvent>;

    /// Get current state
    fn state(&self) -> RecognizerState;

    /// Get the target node ID
    fn target_node_id(&self) -> u32;

    /// Accept the gesture (called by arena)
    fn accept(&mut self);

    /// Reject the gesture (called by arena)
    fn reject(&mut self);

    /// Cancel the gesture
    fn cancel(&mut self);

    /// Whether this recognizer has consumed the given pointer
    fn tracks_pointer(&self, pointer_id: u32) -> bool;

    /// Get the number of pointers being tracked
    fn pointer_count(&self) -> usize;

    /// Whether this recognizer is eligible to win in the arena
    /// 
    /// Called by the arena to determine which recognizers are competing
    fn is_eligible(&self) -> bool;
}

// =============== Tap Gesture Recognizer ===============

/// Recognizes tap gestures (single, double, triple)
pub struct TapGestureRecognizer {
    config: GestureConfig,
    state: RecognizerState,
    /// Pointer being tracked
    tracked_pointer: Option<PointerData>,
    /// Number of taps completed
    tap_count: u32,
    /// Maximum taps to recognize (1=single, 2=double, 3=triple)
    max_taps: u32,
    /// Last tap timestamp for double/triple tap detection
    last_tap_time_us: u64,
    /// Last tap position for double/triple tap detection
    last_tap_x: f32,
    last_tap_y: f32,
}

impl TapGestureRecognizer {
    pub fn new(config: GestureConfig, max_taps: u32) -> Self {
        Self {
            config,
            state: RecognizerState::Possible,
            tracked_pointer: None,
            tap_count: 0,
            max_taps,
            last_tap_time_us: 0,
            last_tap_x: 0.0,
            last_tap_y: 0.0,
        }
    }

    /// Create a single tap recognizer
    pub fn single_tap(config: GestureConfig) -> Self {
        Self::new(config, 1)
    }

    /// Create a double tap recognizer
    pub fn double_tap(config: GestureConfig) -> Self {
        Self::new(config, 2)
    }

    fn reset(&mut self) {
        self.state = RecognizerState::Possible;
        self.tracked_pointer = None;
        self.tap_count = 0;
    }

    fn is_within_tap_slop(&self, pointer: &PointerData) -> bool {
        pointer.distance_from_start() <= self.config.settings.tap_slop
    }

    fn is_within_double_tap_slop(&self, x: f32, y: f32) -> bool {
        let dx = x - self.last_tap_x;
        let dy = y - self.last_tap_y;
        (dx * dx + dy * dy).sqrt() <= self.config.settings.double_tap_slop
    }

    fn is_within_double_tap_time(&self, current_time_us: u64) -> bool {
        let elapsed_ms = (current_time_us - self.last_tap_time_us) / 1000;
        elapsed_ms <= self.config.settings.double_tap_timeout_ms
    }
}

impl GestureRecognizer for TapGestureRecognizer {
    fn handle_event(
        &mut self,
        event: &PointerEvent,
        _tracked_pointers: &std::collections::HashMap<u32, PointerData>,
    ) -> Vec<GestureEvent> {
        let mut events = Vec::new();

        match event.event_type {
            PointerEventType::Down => {
                // Check if this could be a continuation of multi-tap
                if self.tap_count > 0 {
                    if !self.is_within_double_tap_time(event.timestamp_us)
                        || !self.is_within_double_tap_slop(event.x, event.y)
                    {
                        // Too slow or too far - this is a new tap sequence
                        self.tap_count = 0;
                    }
                }

                self.tracked_pointer = Some(PointerData::new(event));
                self.state = RecognizerState::Began;
            }
            PointerEventType::Move => {
                if let Some(ref pointer) = self.tracked_pointer {
                    if pointer.pointer_id == event.pointer_id {
                        let mut updated = *pointer;
                        updated.update(event);

                        // Check if moved too far for a tap
                        if !self.is_within_tap_slop(&updated) {
                            self.state = RecognizerState::Rejected;
                            self.reset();
                        }

                        self.tracked_pointer = Some(updated);
                    }
                }
            }
            PointerEventType::Up => {
                if let Some(ref pointer) = self.tracked_pointer {
                    if pointer.pointer_id == event.pointer_id {
                        // Check if still within tap slop
                        if self.is_within_tap_slop(pointer) {
                            self.tap_count += 1;
                            self.last_tap_time_us = event.timestamp_us;
                            self.last_tap_x = event.x;
                            self.last_tap_y = event.y;

                            if self.tap_count >= self.max_taps {
                                // Completed all required taps
                                self.state = RecognizerState::Accepted;
                                
                                // Generate appropriate event
                                let gesture_event = match self.max_taps {
                                    1 => GestureEvent::tap(
                                        self.config.target_node_id,
                                        event.pointer_id,
                                        event.x,
                                        event.y,
                                        event.timestamp_us,
                                    ),
                                    2 => GestureEvent::double_tap(
                                        self.config.target_node_id,
                                        event.pointer_id,
                                        event.x,
                                        event.y,
                                        event.timestamp_us,
                                    ),
                                    _ => GestureEvent::tap(
                                        self.config.target_node_id,
                                        event.pointer_id,
                                        event.x,
                                        event.y,
                                        event.timestamp_us,
                                    ),
                                };
                                
                                events.push(gesture_event);
                                self.reset();
                            } else {
                                // Waiting for more taps
                                self.tracked_pointer = None;
                                self.state = RecognizerState::Possible;
                            }
                        } else {
                            self.state = RecognizerState::Rejected;
                            self.reset();
                        }
                    }
                }
            }
            PointerEventType::Cancel => {
                self.state = RecognizerState::Cancelled;
                self.reset();
            }
        }

        events
    }

    fn state(&self) -> RecognizerState {
        self.state
    }

    fn target_node_id(&self) -> u32 {
        self.config.target_node_id
    }

    fn accept(&mut self) {
        self.state = RecognizerState::Accepted;
    }

    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
    }

    fn cancel(&mut self) {
        self.state = RecognizerState::Cancelled;
        self.reset();
    }

    fn tracks_pointer(&self, pointer_id: u32) -> bool {
        self.tracked_pointer
            .map(|p| p.pointer_id == pointer_id)
            .unwrap_or(false)
    }

    fn pointer_count(&self) -> usize {
        if self.tracked_pointer.is_some() {
            1
        } else {
            0
        }
    }

    fn is_eligible(&self) -> bool {
        matches!(self.state, RecognizerState::Possible | RecognizerState::Began)
    }
}

// =============== Long Press Gesture Recognizer ===============

/// Recognizes long press gestures
pub struct LongPressGestureRecognizer {
    config: GestureConfig,
    state: RecognizerState,
    tracked_pointer: Option<PointerData>,
    /// Whether long press has triggered
    has_triggered: bool,
    /// Timeout deadline (for checking in handle_event)
    deadline_us: u64,
}

impl LongPressGestureRecognizer {
    pub fn new(config: GestureConfig) -> Self {
        Self {
            config,
            state: RecognizerState::Possible,
            tracked_pointer: None,
            has_triggered: false,
            deadline_us: 0,
        }
    }

    fn reset(&mut self) {
        self.state = RecognizerState::Possible;
        self.tracked_pointer = None;
        self.has_triggered = false;
        self.deadline_us = 0;
    }

    fn is_within_slop(&self, pointer: &PointerData) -> bool {
        pointer.distance_from_start() <= self.config.settings.long_press_slop
    }

    fn check_deadline(&mut self, current_time_us: u64) -> Option<GestureEvent> {
        if self.has_triggered || self.deadline_us == 0 {
            return None;
        }

        if current_time_us >= self.deadline_us {
            if let Some(ref pointer) = self.tracked_pointer {
                if self.is_within_slop(pointer) {
                    self.has_triggered = true;
                    self.state = RecognizerState::Changed;
                    
                    return Some(GestureEvent::long_press_start(
                        self.config.target_node_id,
                        pointer.pointer_id,
                        pointer.current_x,
                        pointer.current_y,
                        current_time_us,
                    ));
                }
            }
        }
        None
    }
}

impl GestureRecognizer for LongPressGestureRecognizer {
    fn handle_event(
        &mut self,
        event: &PointerEvent,
        _tracked_pointers: &std::collections::HashMap<u32, PointerData>,
    ) -> Vec<GestureEvent> {
        let mut events = Vec::new();

        match event.event_type {
            PointerEventType::Down => {
                self.tracked_pointer = Some(PointerData::new(event));
                self.deadline_us = event.timestamp_us 
                    + self.config.settings.long_press_timeout_ms * 1000;
                self.state = RecognizerState::Began;
            }
            PointerEventType::Move => {
                let slop = self.config.settings.long_press_slop;
                let mut should_reject = false;
                let mut should_check_deadline = false;
                
                if let Some(ref mut pointer) = self.tracked_pointer {
                    if pointer.pointer_id == event.pointer_id {
                        pointer.update(event);

                        // Check if moved too far
                        if pointer.distance_from_start() > slop {
                            should_reject = true;
                        } else {
                            should_check_deadline = true;
                        }
                    }
                }
                
                if should_reject {
                    if self.has_triggered {
                        if let Some(ref pointer) = self.tracked_pointer {
                            // Send long press end if was active
                            events.push(GestureEvent::long_press_end(
                                self.config.target_node_id,
                                pointer.pointer_id,
                                pointer.current_x,
                                pointer.current_y,
                                event.timestamp_us,
                            ));
                        }
                    }
                    self.state = RecognizerState::Rejected;
                    self.reset();
                } else if should_check_deadline {
                    if let Some(event) = self.check_deadline(event.timestamp_us) {
                        events.push(event);
                    }
                }
            }
            PointerEventType::Up => {
                if let Some(ref pointer) = self.tracked_pointer {
                    if pointer.pointer_id == event.pointer_id {
                        if self.has_triggered {
                            // Send long press end
                            events.push(GestureEvent::long_press_end(
                                self.config.target_node_id,
                                pointer.pointer_id,
                                pointer.current_x,
                                pointer.current_y,
                                event.timestamp_us,
                            ));
                        }
                        self.state = RecognizerState::Ended;
                        self.reset();
                    }
                }
            }
            PointerEventType::Cancel => {
                if self.has_triggered {
                    if let Some(ref pointer) = self.tracked_pointer {
                        events.push(GestureEvent::long_press_end(
                            self.config.target_node_id,
                            pointer.pointer_id,
                            pointer.current_x,
                            pointer.current_y,
                            event.timestamp_us,
                        ));
                    }
                }
                self.state = RecognizerState::Cancelled;
                self.reset();
            }
        }

        // Check deadline on every event (in case no move events come in time)
        if let Some(event) = self.check_deadline(event.timestamp_us) {
            events.push(event);
        }

        events
    }

    fn state(&self) -> RecognizerState {
        self.state
    }

    fn target_node_id(&self) -> u32 {
        self.config.target_node_id
    }

    fn accept(&mut self) {
        self.state = RecognizerState::Accepted;
    }

    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
    }

    fn cancel(&mut self) {
        self.state = RecognizerState::Cancelled;
        self.reset();
    }

    fn tracks_pointer(&self, pointer_id: u32) -> bool {
        self.tracked_pointer
            .map(|p| p.pointer_id == pointer_id)
            .unwrap_or(false)
    }

    fn pointer_count(&self) -> usize {
        if self.tracked_pointer.is_some() {
            1
        } else {
            0
        }
    }

    fn is_eligible(&self) -> bool {
        matches!(self.state, RecognizerState::Possible | RecognizerState::Began)
    }
}

// =============== Pan Gesture Recognizer ===============

/// Recognizes pan (drag) gestures
pub struct PanGestureRecognizer {
    config: GestureConfig,
    state: RecognizerState,
    tracked_pointer: Option<PointerData>,
    /// Whether pan has started
    has_started: bool,
    /// Last reported position for delta calculation
    last_x: f32,
    last_y: f32,
}

impl PanGestureRecognizer {
    pub fn new(config: GestureConfig) -> Self {
        Self {
            config,
            state: RecognizerState::Possible,
            tracked_pointer: None,
            has_started: false,
            last_x: 0.0,
            last_y: 0.0,
        }
    }

    fn reset(&mut self) {
        self.state = RecognizerState::Possible;
        self.tracked_pointer = None;
        self.has_started = false;
        self.last_x = 0.0;
        self.last_y = 0.0;
    }

    #[allow(dead_code)]
    fn slop_met(&self, pointer: &PointerData) -> bool {
        pointer.distance_from_start() >= self.config.settings.pan_slop
    }
}

impl GestureRecognizer for PanGestureRecognizer {
    fn handle_event(
        &mut self,
        event: &PointerEvent,
        _tracked_pointers: &std::collections::HashMap<u32, PointerData>,
    ) -> Vec<GestureEvent> {
        let mut events = Vec::new();

        match event.event_type {
            PointerEventType::Down => {
                self.tracked_pointer = Some(PointerData::new(event));
                self.state = RecognizerState::Began;
            }
            PointerEventType::Move => {
                let pan_slop = self.config.settings.pan_slop;
                let mut should_start_pan = false;
                let mut should_update_pan = false;
                let mut delta_x = 0.0;
                let mut delta_y = 0.0;
                
                if let Some(ref mut pointer) = self.tracked_pointer {
                    if pointer.pointer_id == event.pointer_id {
                        pointer.update(event);

                        if !self.has_started {
                            if pointer.distance_from_start() >= pan_slop {
                                should_start_pan = true;
                            }
                        } else {
                            should_update_pan = true;
                            delta_x = pointer.current_x - self.last_x;
                            delta_y = pointer.current_y - self.last_y;
                            self.last_x = pointer.current_x;
                            self.last_y = pointer.current_y;
                        }
                    }
                }
                
                if should_start_pan {
                    if let Some(ref pointer) = self.tracked_pointer {
                        self.has_started = true;
                        self.state = RecognizerState::Accepted;
                        self.last_x = pointer.current_x;
                        self.last_y = pointer.current_y;

                        events.push(GestureEvent::pan_start(
                            self.config.target_node_id,
                            pointer.pointer_id,
                            pointer.current_x,
                            pointer.current_y,
                            event.timestamp_us,
                        ));
                    }
                } else if should_update_pan {
                    if let Some(ref pointer) = self.tracked_pointer {
                        events.push(GestureEvent::pan_update(
                            self.config.target_node_id,
                            pointer.pointer_id,
                            pointer.current_x,
                            pointer.current_y,
                            delta_x,
                            delta_y,
                            event.timestamp_us,
                        ));
                    }
                }
            }
            PointerEventType::Up => {
                if let Some(ref pointer) = self.tracked_pointer {
                    if pointer.pointer_id == event.pointer_id {
                        if self.has_started {
                            // End pan
                            let velocity_x = pointer.current_x - self.last_x;
                            let velocity_y = pointer.current_y - self.last_y;
                            
                            events.push(GestureEvent::pan_end(
                                self.config.target_node_id,
                                pointer.pointer_id,
                                pointer.current_x,
                                pointer.current_y,
                                velocity_x,
                                velocity_y,
                                event.timestamp_us,
                            ));
                        } else {
                            // Didn't move enough to be a pan
                            self.state = RecognizerState::Rejected;
                        }
                        self.reset();
                    }
                }
            }
            PointerEventType::Cancel => {
                if self.has_started {
                    if let Some(ref pointer) = self.tracked_pointer {
                        events.push(GestureEvent::pan_end(
                            self.config.target_node_id,
                            pointer.pointer_id,
                            pointer.current_x,
                            pointer.current_y,
                            0.0,
                            0.0,
                            event.timestamp_us,
                        ));
                    }
                }
                self.state = RecognizerState::Cancelled;
                self.reset();
            }
        }

        events
    }

    fn state(&self) -> RecognizerState {
        self.state
    }

    fn target_node_id(&self) -> u32 {
        self.config.target_node_id
    }

    fn accept(&mut self) {
        if !self.has_started {
            self.state = RecognizerState::Accepted;
        }
    }

    fn reject(&mut self) {
        self.state = RecognizerState::Rejected;
    }

    fn cancel(&mut self) {
        self.state = RecognizerState::Cancelled;
        self.reset();
    }

    fn tracks_pointer(&self, pointer_id: u32) -> bool {
        self.tracked_pointer
            .map(|p| p.pointer_id == pointer_id)
            .unwrap_or(false)
    }

    fn pointer_count(&self) -> usize {
        if self.tracked_pointer.is_some() {
            1
        } else {
            0
        }
    }

    fn is_eligible(&self) -> bool {
        matches!(self.state, RecognizerState::Possible | RecognizerState::Began)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_pointer_event(
        event_type: PointerEventType,
        x: f32,
        y: f32,
        timestamp_us: u64,
    ) -> PointerEvent {
        PointerEvent {
            event_type,
            pointer_id: 0,
            timestamp_us,
            x,
            y,
            pressure: 1.0,
            target_node_id: 1,
        }
    }

    fn create_config() -> GestureConfig {
        GestureConfig {
            settings: GestureSettings::default(),
            target_node_id: 1,
        }
    }

    #[test]
    fn test_tap_recognizer() {
        let config = create_config();
        let mut recognizer = TapGestureRecognizer::single_tap(config);
        let empty_pointers = HashMap::new();

        // Down
        let down = create_pointer_event(PointerEventType::Down, 100.0, 100.0, 0);
        let events = recognizer.handle_event(&down, &empty_pointers);
        assert!(events.is_empty());
        assert!(matches!(recognizer.state(), RecognizerState::Began));

        // Up (tap)
        let up = create_pointer_event(PointerEventType::Up, 100.0, 100.0, 100_000);
        let events = recognizer.handle_event(&up, &empty_pointers);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, crate::events::GestureEventType::Tap));
    }

    #[test]
    fn test_tap_recognizer_reject_on_move() {
        let config = create_config();
        let mut recognizer = TapGestureRecognizer::single_tap(config);
        let empty_pointers = HashMap::new();

        // Down
        let down = create_pointer_event(PointerEventType::Down, 100.0, 100.0, 0);
        recognizer.handle_event(&down, &empty_pointers);

        // Move far (should reject)
        let move_far = create_pointer_event(PointerEventType::Move, 200.0, 200.0, 50_000);
        let events = recognizer.handle_event(&move_far, &empty_pointers);
        assert!(events.is_empty());
        assert!(matches!(recognizer.state(), RecognizerState::Rejected));
    }

    #[test]
    fn test_long_press_recognizer() {
        let config = create_config();
        let mut recognizer = LongPressGestureRecognizer::new(config);
        let empty_pointers = HashMap::new();

        // Down
        let down = create_pointer_event(PointerEventType::Down, 100.0, 100.0, 0);
        let events = recognizer.handle_event(&down, &empty_pointers);
        assert!(events.is_empty());

        // Wait for timeout (500ms = 500000us)
        let timeout = create_pointer_event(PointerEventType::Move, 100.0, 100.0, 600_000);
        let events = recognizer.handle_event(&timeout, &empty_pointers);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, crate::events::GestureEventType::LongPressStart));

        // Up
        let up = create_pointer_event(PointerEventType::Up, 100.0, 100.0, 700_000);
        let events = recognizer.handle_event(&up, &empty_pointers);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, crate::events::GestureEventType::LongPressEnd));
    }

    #[test]
    fn test_pan_recognizer() {
        let config = create_config();
        let mut recognizer = PanGestureRecognizer::new(config);
        let empty_pointers = HashMap::new();

        // Down
        let down = create_pointer_event(PointerEventType::Down, 100.0, 100.0, 0);
        let events = recognizer.handle_event(&down, &empty_pointers);
        assert!(events.is_empty());

        // Small move (within slop)
        let small_move = create_pointer_event(PointerEventType::Move, 105.0, 105.0, 16_000);
        let events = recognizer.handle_event(&small_move, &empty_pointers);
        assert!(events.is_empty());

        // Large move (past slop)
        let large_move = create_pointer_event(PointerEventType::Move, 130.0, 130.0, 32_000);
        let events = recognizer.handle_event(&large_move, &empty_pointers);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, crate::events::GestureEventType::PanStart));

        // Continue pan
        let pan_move = create_pointer_event(PointerEventType::Move, 140.0, 140.0, 48_000);
        let events = recognizer.handle_event(&pan_move, &empty_pointers);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, crate::events::GestureEventType::PanUpdate));

        // Up
        let up = create_pointer_event(PointerEventType::Up, 140.0, 140.0, 64_000);
        let events = recognizer.handle_event(&up, &empty_pointers);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, crate::events::GestureEventType::PanEnd));
    }
}
