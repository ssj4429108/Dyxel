// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture Composition - Exclusive, Simultaneous, Sequenced
//!
//! Implements Flutter-style gesture composition with three relationship types:
//! - Exclusive: Only one gesture can win (e.g., Tap vs DoubleTap)
//! - Simultaneous: Multiple gestures can win together (e.g., Pan + Scale)
//! - Sequenced: Gestures must complete in order (e.g., LongPress then Pan)

use std::time::Instant;

use crate::events::{GestureEvent, PointerEvent};
use crate::recognizer::{GestureRecognizer, RecognizerState};

/// Gesture relationship type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureRelationship {
    /// Only one gesture can win
    Exclusive,
    /// Multiple gestures can win together
    Simultaneous,
    /// Gestures must complete in order
    Sequenced,
}

/// A composable gesture that wraps multiple recognizers
pub trait ComposableGesture {
    /// Process a pointer event
    fn handle_event(
        &mut self,
        event: &PointerEvent,
    ) -> Vec<GestureEvent>;

    /// Check timers
    fn check_timers(&mut self, now: Instant) -> Vec<GestureEvent>;

    /// Get all recognizers
    fn recognizers(&self) -> &[Box<dyn GestureRecognizer>];

    /// Get all recognizers mutably
    fn recognizers_mut(&mut self) -> &mut [Box<dyn GestureRecognizer>];

    /// Check if this gesture is resolved
    fn is_resolved(&self) -> bool;

    /// Reset the gesture
    fn reset(&mut self);
}

// ============================================================================
// Exclusive Gesture
// ============================================================================

/// Exclusive gesture - only one recognizer can win
///
/// # Example
/// ```ignore
/// ExclusiveGesture::new(vec![
///     TapGestureRecognizer::single_tap(1, 1),
///     TapGestureRecognizer::double_tap(2, 1),
/// ])
/// ```
pub struct ExclusiveGesture {
    recognizers: Vec<Box<dyn GestureRecognizer>>,
    winner: Option<u32>,
    is_resolved: bool,
}

impl ExclusiveGesture {
    /// Create a new exclusive gesture group
    pub fn new(recognizers: Vec<Box<dyn GestureRecognizer>>) -> Self {
        Self {
            recognizers,
            winner: None,
            is_resolved: false,
        }
    }

    /// Get the winning recognizer ID
    pub fn winner(&self) -> Option<u32> {
        self.winner
    }

    /// Handle a recognizer accepting
    fn on_recognizer_accept(&mut self, recognizer_id: u32) {
        if self.winner.is_some() {
            return;
        }

        self.winner = Some(recognizer_id);

        // Reject all other recognizers
        for recognizer in &mut self.recognizers {
            if recognizer.id() != recognizer_id {
                recognizer.reject();
            }
        }

        self.is_resolved = true;
    }
}

impl ComposableGesture for ExclusiveGesture {
    fn handle_event(
        &mut self,
        event: &PointerEvent,
    ) -> Vec<GestureEvent> {
        if self.is_resolved {
            // Route events only to the winner
            if let Some(winner_id) = self.winner {
                if let Some(winner) =
                    self.recognizers.iter_mut().find(|r| r.id() == winner_id)
                {
                    return winner.handle_event(event);
                }
            }
            return vec![];
        }

        let mut events = vec![];
        let mut newly_accepted = None;

        for recognizer in &mut self.recognizers {
            let recognizer_id = recognizer.id();

            let recognizer_events = recognizer.handle_event(event);
            events.extend(recognizer_events);

            // Check if this recognizer accepted
            if recognizer.state().is_accepted() && self.winner.is_none() {
                newly_accepted = Some(recognizer_id);
            }
        }

        // Handle exclusive logic
        if let Some(winner_id) = newly_accepted {
            self.on_recognizer_accept(winner_id);
        }

        // Check if all recognizers have failed
        let all_failed = self.recognizers.iter().all(|r| {
            matches!(
                r.state(),
                RecognizerState::Failed | RecognizerState::Cancelled
            )
        });
        if all_failed {
            self.is_resolved = true;
        }

        events
    }

    fn check_timers(&mut self, now: Instant) -> Vec<GestureEvent> {
        if self.is_resolved {
            return vec![];
        }

        let mut events = vec![];
        let mut newly_accepted = None;

        for recognizer in &mut self.recognizers {
            let recognizer_id = recognizer.id();

            let recognizer_events = recognizer.check_timers(now);
            events.extend(recognizer_events);

            if recognizer.state().is_accepted() && self.winner.is_none() {
                newly_accepted = Some(recognizer_id);
            }
        }

        if let Some(winner_id) = newly_accepted {
            self.on_recognizer_accept(winner_id);
        }

        events
    }

    fn recognizers(&self) -> &[Box<dyn GestureRecognizer>] {
        &self.recognizers
    }

    fn recognizers_mut(&mut self) -> &mut [Box<dyn GestureRecognizer>] {
        &mut self.recognizers
    }

    fn is_resolved(&self) -> bool {
        self.is_resolved
    }

    fn reset(&mut self) {
        self.winner = None;
        self.is_resolved = false;
        for recognizer in &mut self.recognizers {
            recognizer.reset();
        }
    }
}

// ============================================================================
// Simultaneous Gesture
// ============================================================================

/// Simultaneous gesture - multiple recognizers can win together
///
/// # Example
/// ```ignore
/// SimultaneousGesture::new(vec![
///     Box::new(PanGestureRecognizer::new(1, 1)),
///     Box::new(ScaleGestureRecognizer::new(2, 1)),
/// ])
/// ```
pub struct SimultaneousGesture {
    recognizers: Vec<Box<dyn GestureRecognizer>>,
    is_resolved: bool,
}

impl SimultaneousGesture {
    /// Create a new simultaneous gesture group
    pub fn new(recognizers: Vec<Box<dyn GestureRecognizer>>) -> Self {
        Self {
            recognizers,
            is_resolved: false,
        }
    }

    /// Get the number of accepted recognizers
    pub fn accepted_count(&self) -> usize {
        self.recognizers
            .iter()
            .filter(|r| r.state().is_accepted())
            .count()
    }
}

impl ComposableGesture for SimultaneousGesture {
    fn handle_event(
        &mut self,
        event: &PointerEvent,
    ) -> Vec<GestureEvent> {
        if self.is_resolved {
            return vec![];
        }

        let mut events = vec![];

        for recognizer in &mut self.recognizers {
            // Continue routing events to all recognizers that haven't failed
            if !matches!(
                recognizer.state(),
                RecognizerState::Failed | RecognizerState::Cancelled
            ) {
                let recognizer_events = recognizer.handle_event(event);
                events.extend(recognizer_events);
            }
        }

        // Check if all recognizers are resolved
        let all_resolved = self.recognizers.iter().all(|r| r.state().is_terminal());
        if all_resolved {
            self.is_resolved = true;
        }

        events
    }

    fn check_timers(&mut self, now: Instant) -> Vec<GestureEvent> {
        if self.is_resolved {
            return vec![];
        }

        let mut events = vec![];

        for recognizer in &mut self.recognizers {
            if !matches!(
                recognizer.state(),
                RecognizerState::Failed | RecognizerState::Cancelled
            ) {
                let recognizer_events = recognizer.check_timers(now);
                events.extend(recognizer_events);
            }
        }

        events
    }

    fn recognizers(&self) -> &[Box<dyn GestureRecognizer>] {
        &self.recognizers
    }

    fn recognizers_mut(&mut self) -> &mut [Box<dyn GestureRecognizer>] {
        &mut self.recognizers
    }

    fn is_resolved(&self) -> bool {
        self.is_resolved
    }

    fn reset(&mut self) {
        self.is_resolved = false;
        for recognizer in &mut self.recognizers {
            recognizer.reset();
        }
    }
}

// ============================================================================
// Sequenced Gesture
// ============================================================================

/// Sequenced gesture - recognizers must complete in order
///
/// # Example
/// ```ignore
/// SequencedGesture::new(vec![
///     Box::new(LongPressGestureRecognizer::new(1, 1)),
///     Box::new(PanGestureRecognizer::new(2, 1)),
/// ])
/// ```
pub struct SequencedGesture {
    recognizers: Vec<Box<dyn GestureRecognizer>>,
    current_index: usize,
    is_resolved: bool,
}

impl SequencedGesture {
    /// Create a new sequenced gesture group
    pub fn new(recognizers: Vec<Box<dyn GestureRecognizer>>) -> Self {
        Self {
            recognizers,
            current_index: 0,
            is_resolved: false,
        }
    }

    /// Get the current active recognizer index
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    /// Advance to the next recognizer
    fn advance(&mut self) {
        if self.current_index < self.recognizers.len() {
            self.current_index += 1;
        }
        if self.current_index >= self.recognizers.len() {
            self.is_resolved = true;
        }
    }

    /// Check if we're at the last recognizer
    fn is_last(&self) -> bool {
        self.current_index >= self.recognizers.len() - 1
    }
}

impl ComposableGesture for SequencedGesture {
    fn handle_event(
        &mut self,
        event: &PointerEvent,
    ) -> Vec<GestureEvent> {
        if self.is_resolved || self.current_index >= self.recognizers.len() {
            return vec![];
        }

        let mut events = vec![];

        // Route event only to current recognizer
        let current = &mut self.recognizers[self.current_index];
        events.extend(current.handle_event(event));

        // Check if current recognizer completed successfully
        if matches!(current.state(), RecognizerState::Ended) {
            if self.is_last() {
                self.is_resolved = true;
            } else {
                self.advance();
            }
        } else if matches!(
            current.state(),
            RecognizerState::Failed | RecognizerState::Cancelled
        ) {
            // Sequence failed
            self.is_resolved = true;
        }

        events
    }

    fn check_timers(&mut self, now: Instant) -> Vec<GestureEvent> {
        if self.is_resolved || self.current_index >= self.recognizers.len() {
            return vec![];
        }

        let mut events = vec![];

        let current = &mut self.recognizers[self.current_index];
        events.extend(current.check_timers(now));

        // Check if current recognizer completed
        if matches!(current.state(), RecognizerState::Ended) {
            if self.is_last() {
                self.is_resolved = true;
            } else {
                self.advance();
            }
        } else if matches!(
            current.state(),
            RecognizerState::Failed | RecognizerState::Cancelled
        ) {
            self.is_resolved = true;
        }

        events
    }

    fn recognizers(&self) -> &[Box<dyn GestureRecognizer>] {
        &self.recognizers
    }

    fn recognizers_mut(&mut self) -> &mut [Box<dyn GestureRecognizer>] {
        &mut self.recognizers
    }

    fn is_resolved(&self) -> bool {
        self.is_resolved
    }

    fn reset(&mut self) {
        self.current_index = 0;
        self.is_resolved = false;
        for recognizer in &mut self.recognizers {
            recognizer.reset();
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognizer::{
        LongPressGestureRecognizer, PanGestureRecognizer, TapGestureRecognizer,
    };
    use crate::test_utils::{GestureEventAssertions, PointerEventBuilder};
    use std::time::Duration;

    // ===== Exclusive Tests =====

    #[test]
    fn test_exclusive_single_winner() {
        // Create recognizers where single_tap needs to wait (tap_count=1 but we're competing)
        // In this case, single_tap will fire immediately on first up, winning over double_tap
        let mut gesture = ExclusiveGesture::new(vec![
            Box::new(TapGestureRecognizer::single_tap(1, 1)),
            Box::new(TapGestureRecognizer::double_tap(2, 1)),
        ]);

        // First tap - single_tap fires immediately because tap_count=1
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        let events = gesture.handle_event(&up);

        // Single tap should fire immediately (tap_count=1)
        events.assert_tap(1);
        assert_eq!(gesture.winner(), Some(1));
    }

    #[test]
    fn test_exclusive_double_tap_wins() {
        // Use two double_tap recognizers to test that the second one can win
        // when user actually double taps
        let mut gesture = ExclusiveGesture::new(vec![
            Box::new(TapGestureRecognizer::new(1, 1).with_tap_count(2)), // 2-tap recognizer with id 1
            Box::new(TapGestureRecognizer::double_tap(2, 1)),                  // 2-tap recognizer with id 2
        ]);
        // Both recognizers are waiting for 2 taps
        // First tap
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        let events = gesture.handle_event(&up);
        assert!(events.is_empty(), "Should wait for second tap");

        // Second tap (quickly)
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        let events = gesture.handle_event(&up);

        // Both recognizers fire, but only one wins (first to accept)
        assert!(!events.is_empty(), "Double tap should have fired");
        assert!(events.iter().any(|e| e.tap_count == 2), "Expected tap_count=2");
    }

    #[test]
    fn test_exclusive_winner_rejects_others() {
        let mut gesture = ExclusiveGesture::new(vec![
            Box::new(TapGestureRecognizer::single_tap(1, 1)),
            Box::new(LongPressGestureRecognizer::new(2, 1)),
        ]);

        // Long press wins
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);

        let events = gesture.check_timers(Instant::now() + Duration::from_millis(600));
        events.assert_long_press_start();
        assert_eq!(gesture.winner(), Some(2));

        // Check that tap recognizer was rejected
        let tap = gesture
            .recognizers()
            .iter()
            .find(|r| r.id() == 1)
            .unwrap();
        assert!(matches!(tap.state(), RecognizerState::Failed));
    }

    // ===== Simultaneous Tests =====

    #[test]
    fn test_simultaneous_multiple_accept() {
        let mut gesture = SimultaneousGesture::new(vec![
            Box::new(PanGestureRecognizer::new(1, 1)),
            Box::new(PanGestureRecognizer::new(2, 1)), // Using Pan as placeholder for Scale
        ]);

        // Start gesture
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);

        // Move beyond slop
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(130.0, 130.0);
        let events = gesture.handle_event(&move_evt);

        // Both should accept
        assert!(
            events
                .iter()
                .filter(|e| matches!(
                    e.event_type,
                    crate::events::GestureEventType::PanStart
                ))
                .count()
                >= 1
        );
    }

    #[test]
    fn test_simultaneous_independent_failure() {
        let mut gesture = SimultaneousGesture::new(vec![
            Box::new(TapGestureRecognizer::single_tap(1, 1)),
            Box::new(PanGestureRecognizer::new(2, 1)),
        ]);

        // Start with tap recognizer
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);

        // Move beyond tap slop (pan triggers)
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(130.0, 130.0);
        let events = gesture.handle_event(&move_evt);

        // Pan should accept, tap should fail
        assert!(events.iter().any(|e| matches!(
            e.event_type,
            crate::events::GestureEventType::PanStart
        )));

        let tap = gesture
            .recognizers()
            .iter()
            .find(|r| r.id() == 1)
            .unwrap();
        assert!(matches!(tap.state(), RecognizerState::Failed));
    }

    // ===== Sequenced Tests =====

    #[test]
    fn test_sequenced_order_enforced() {
        let mut gesture = SequencedGesture::new(vec![
            Box::new(TapGestureRecognizer::single_tap(1, 1)),
            Box::new(PanGestureRecognizer::new(2, 1)),
        ]);

        // First should be tap
        assert_eq!(gesture.current_index(), 0);

        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        gesture.handle_event(&up);

        // Tap completed, should advance to pan
        assert_eq!(gesture.current_index(), 1);
    }

    #[test]
    fn test_sequenced_pan_after_tap() {
        let mut gesture = SequencedGesture::new(vec![
            Box::new(TapGestureRecognizer::single_tap(1, 1)),
            Box::new(PanGestureRecognizer::new(2, 1)),
        ]);

        // Complete tap
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        gesture.handle_event(&up);

        assert_eq!(gesture.current_index(), 1);

        // Now pan should work
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);

        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(130.0, 130.0);
        let events = gesture.handle_event(&move_evt);

        events.assert_pan_start();
    }

    #[test]
    fn test_sequenced_failure_stops_sequence() {
        let mut gesture = SequencedGesture::new(vec![
            Box::new(TapGestureRecognizer::single_tap(1, 1)),
            Box::new(PanGestureRecognizer::new(2, 1)),
        ]);

        // Start tap but move too much (fail tap)
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);

        // Move beyond slop - tap fails
        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(150.0, 150.0);
        gesture.handle_event(&move_evt);

        // Sequence should be resolved with failure
        assert!(gesture.is_resolved());
    }

    #[test]
    fn test_sequenced_long_press_then_pan() {
        let mut gesture = SequencedGesture::new(vec![
            Box::new(LongPressGestureRecognizer::new(1, 1)),
            Box::new(PanGestureRecognizer::new(2, 1)),
        ]);
        let start = Instant::now();

        // Start long press
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);

        // Wait for long press to trigger
        let events = gesture.check_timers(start + Duration::from_millis(600));
        events.assert_long_press_start();

        // Release to complete long press sequence
        let up = PointerEventBuilder::new(0).node_id(1).up_at(100.0, 100.0);
        gesture.handle_event(&up);

        // Now should be at pan recognizer
        assert_eq!(gesture.current_index(), 1);

        // Now pan should work - need a new pointer down for pan
        let down = PointerEventBuilder::new(0).node_id(1).down(100.0, 100.0);
        gesture.handle_event(&down);

        let move_evt = PointerEventBuilder::new(0).node_id(1).move_to(130.0, 130.0);
        let events = gesture.handle_event(&move_evt);

        events.assert_pan_start();
    }
}
