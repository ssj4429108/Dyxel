// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture Arena V3 - Flutter-compatible arena system
//!
//! Implements the GestureArena pattern from Flutter where multiple
//! gesture recognizers compete to handle a single pointer sequence.

use std::collections::HashMap;
use std::time::Instant;

use crate::events::{GestureEvent, PointerEvent, PointerEventType};
use crate::recognizer::{GestureDisposition, GestureRecognizer, RecognizerState};

/// Unique identifier for an arena
pub type ArenaId = u64;

/// A member of the gesture arena
pub trait GestureArenaMember: GestureRecognizer {
    /// Called when this member should attempt to accept the gesture
    fn try_accept(&mut self);

    /// Called when this member is rejected by the arena
    fn on_reject(&mut self);

    /// Get the disposition for this member
    fn disposition(&self) -> GestureDisposition;
}

/// Single gesture arena for one pointer
pub struct GestureArena {
    id: ArenaId,
    pointer_id: u32,
    node_id: u32,
    members: Vec<Box<dyn GestureRecognizer>>,
    accepted: Vec<u32>,
    rejected: Vec<u32>,
    is_closed: bool,
}

impl GestureArena {
    /// Create a new arena
    pub fn new(id: ArenaId, pointer_id: u32, node_id: u32) -> Self {
        Self {
            id,
            pointer_id,
            node_id,
            members: Vec::new(),
            accepted: Vec::new(),
            rejected: Vec::new(),
            is_closed: false,
        }
    }

    /// Get arena ID
    pub fn id(&self) -> ArenaId {
        self.id
    }

    /// Get pointer ID
    pub fn pointer_id(&self) -> u32 {
        self.pointer_id
    }

    /// Get node ID
    pub fn node_id(&self) -> u32 {
        self.node_id
    }

    /// Check if arena is closed
    pub fn is_closed(&self) -> bool {
        self.is_closed
    }

    /// Check if any member is waiting for multi-tap
    pub fn has_pending_multi_tap(&self) -> bool {
        self.members.iter().any(|m| {
            // Check if this is a tap recognizer waiting for more taps
            if let Some(tap) = m
                .as_any()
                .downcast_ref::<crate::recognizer::TapGestureRecognizer>()
            {
                return tap.is_waiting_for_more_taps();
            }
            false
        })
    }

    /// Check if all members are resolved (Ended, Failed, or Cancelled)
    pub fn all_members_resolved(&self) -> bool {
        self.members.iter().all(|m| m.state().is_terminal())
    }

    /// Get members reference (for router integration)
    pub fn members(&self) -> &[Box<dyn GestureRecognizer>] {
        &self.members
    }

    /// Add a member to the arena
    pub fn add_member(&mut self, member: Box<dyn GestureRecognizer>) {
        if !self.is_closed {
            self.members.push(member);
        }
    }

    /// Process a pointer event
    pub fn process_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        if self.is_closed {
            log::info!(
                "Arena {}: Closed, ignoring event {:?}",
                self.id,
                event.event_type
            );
            return vec![];
        }

        log::info!(
            "Arena {}: Processing {:?} with {} members",
            self.id,
            event.event_type,
            self.members.len()
        );

        let mut events = vec![];
        let mut newly_accepted = vec![];
        let mut newly_rejected = vec![];

        for member in &mut self.members {
            let member_id = member.id();

            // Skip rejected members (they're out of the competition)
            if self.rejected.contains(&member_id) {
                continue;
            }

            // For accepted members, only skip if they're in terminal state (discrete gestures)
            // Continuous gestures (Pan) need to keep receiving events even after being accepted
            if self.accepted.contains(&member_id) && member.state().is_terminal() {
                continue;
            }

            let member_events = member.handle_event(event);
            events.extend(member_events);

            // Check state transitions
            match member.state() {
                RecognizerState::Began | RecognizerState::Ended => {
                    if !self.accepted.contains(&member_id) {
                        newly_accepted.push(member_id);
                    }
                }
                RecognizerState::Failed | RecognizerState::Cancelled => {
                    if !self.rejected.contains(&member_id) {
                        newly_rejected.push(member_id);
                    }
                }
                _ => {}
            }
        }

        // Handle newly accepted members
        for member_id in newly_accepted {
            self.handle_accept(member_id);
        }

        // Handle newly rejected members
        for member_id in newly_rejected {
            self.handle_reject(member_id);
        }

        // Note: Don't auto-close arena here - let tick() or explicit close handle it
        // This ensures:
        // - Continuous gestures (Pan) can continue receiving events after being accepted
        // - Discrete gestures (Tap) can complete via timer (e.g., multi-tap waiting)

        if !events.is_empty() {
            log::info!(
                "Arena {}: Generated {} events: {:?}",
                self.id,
                events.len(),
                events
                    .iter()
                    .map(|e| format!("{:?}", e.event_type))
                    .collect::<Vec<_>>()
            );
        }

        events
    }

    /// Check timers
    pub fn check_timers(&mut self, now: Instant) -> Vec<GestureEvent> {
        if self.is_closed {
            return vec![];
        }

        let mut events = vec![];
        let mut newly_accepted = vec![];
        let mut newly_rejected = vec![];

        for member in &mut self.members {
            let member_id = member.id();

            // Skip already resolved members
            if self.accepted.contains(&member_id) || self.rejected.contains(&member_id) {
                continue;
            }

            let member_events = member.check_timers(now);
            events.extend(member_events);

            // Check state transitions
            match member.state() {
                RecognizerState::Began | RecognizerState::Ended => {
                    if !self.accepted.contains(&member_id) {
                        newly_accepted.push(member_id);
                    }
                }
                RecognizerState::Failed | RecognizerState::Cancelled => {
                    if !self.rejected.contains(&member_id) {
                        newly_rejected.push(member_id);
                    }
                }
                _ => {}
            }
        }

        // Handle newly accepted members
        for member_id in newly_accepted {
            self.handle_accept(member_id);
        }

        // Handle newly rejected members
        for member_id in newly_rejected {
            self.handle_reject(member_id);
        }

        // Check if arena should close
        self.try_close();

        events
    }

    /// Handle a member accepting
    fn handle_accept(&mut self, member_id: u32) {
        if self.accepted.contains(&member_id) {
            return;
        }

        self.accepted.push(member_id);

        // Accept the member
        if let Some(member) = self.members.iter_mut().find(|m| m.id() == member_id) {
            member.accept();
        }

        // Reject exclusive members
        for other_id in self.get_exclusive_members(member_id) {
            if other_id != member_id && !self.rejected.contains(&other_id) {
                self.handle_reject(other_id);
            }
        }
    }

    /// Handle a member rejecting
    fn handle_reject(&mut self, member_id: u32) {
        if self.rejected.contains(&member_id) {
            return;
        }

        self.rejected.push(member_id);

        if let Some(member) = self.members.iter_mut().find(|m| m.id() == member_id) {
            member.reject();
        }
    }

    /// Get members that are exclusive with the given member
    fn get_exclusive_members(&self, member_id: u32) -> Vec<u32> {
        let mut exclusive = vec![];

        if let Some(member) = self.members.iter().find(|m| m.id() == member_id) {
            for other in &self.members {
                let other_id = other.id();
                // Use category-based exclusivity check
                if other_id != member_id && member.is_exclusive_with(other.as_ref()) {
                    exclusive.push(other_id);
                }
            }
        }

        exclusive
    }

    /// Try to close the arena
    /// Only closes when all members have reached a terminal state (Ended, Failed, or Cancelled)
    fn try_close(&mut self) {
        if self.is_closed {
            return;
        }

        // Don't close if arena has no members (recognizers not added yet)
        if self.members.is_empty() {
            return;
        }

        // Only close if all members are in terminal state
        // This ensures continuous gestures (Pan) can keep receiving events after being accepted
        if self.all_members_resolved() {
            self.is_closed = true;
        }
    }

    /// Force close the arena
    pub fn close(&mut self) {
        self.is_closed = true;
    }

    /// Get the winning member (first accepted)
    pub fn winner(&self) -> Option<u32> {
        self.accepted.first().copied()
    }

    /// Check if a member has been accepted
    pub fn is_accepted(&self, member_id: u32) -> bool {
        self.accepted.contains(&member_id)
    }

    /// Get debug info
    pub fn debug_info(&self) -> String {
        format!(
            "Arena {}: ptr={}, node={}, members={}, accepted={:?}, rejected={:?}, closed={}",
            self.id,
            self.pointer_id,
            self.node_id,
            self.members.len(),
            self.accepted,
            self.rejected,
            self.is_closed
        )
    }
}

/// Manages all gesture arenas
pub struct GestureArenaManager {
    arenas: HashMap<ArenaId, GestureArena>,
    /// Maps pointer_id to arena_id (public for router integration)
    pub pointer_to_arena: HashMap<u32, ArenaId>,
    next_arena_id: ArenaId,
}

impl GestureArenaManager {
    /// Create a new arena manager
    pub fn new() -> Self {
        Self {
            arenas: HashMap::new(),
            pointer_to_arena: HashMap::new(),
            next_arena_id: 1,
        }
    }

    /// Get or create an arena for a pointer
    pub fn get_or_create_arena(&mut self, pointer_id: u32, node_id: u32) -> &mut GestureArena {
        let arena_id = *self.pointer_to_arena.get(&pointer_id).unwrap_or(&0);

        if arena_id == 0 || !self.arenas.contains_key(&arena_id) {
            let new_id = self.next_arena_id;
            self.next_arena_id += 1;

            let arena = GestureArena::new(new_id, pointer_id, node_id);
            self.arenas.insert(new_id, arena);
            self.pointer_to_arena.insert(pointer_id, new_id);

            self.arenas.get_mut(&new_id).unwrap()
        } else {
            self.arenas.get_mut(&arena_id).unwrap()
        }
    }

    /// Get an existing arena
    pub fn get_arena(&self, arena_id: ArenaId) -> Option<&GestureArena> {
        self.arenas.get(&arena_id)
    }

    /// Get an existing arena mutably
    pub fn get_arena_mut(&mut self, arena_id: ArenaId) -> Option<&mut GestureArena> {
        self.arenas.get_mut(&arena_id)
    }

    /// Handle a pointer event
    pub fn handle_pointer_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent> {
        match event.event_type {
            PointerEventType::Down => {
                // Create new arena for this pointer
                let arena = self.get_or_create_arena(event.pointer_id, event.target_node_id);
                arena.process_event(event)
            }
            PointerEventType::Move | PointerEventType::Up | PointerEventType::Cancel => {
                // Route to existing arena
                if let Some(&arena_id) = self.pointer_to_arena.get(&event.pointer_id) {
                    if let Some(arena) = self.arenas.get_mut(&arena_id) {
                        let events = arena.process_event(event);

                        // For discrete gestures (Tap), close arena immediately on Up if resolved
                        // For continuous gestures (Pan), keep arena open until Up event
                        // For multi-tap, keep arena open until all taps are received or timeout
                        if matches!(
                            event.event_type,
                            PointerEventType::Up | PointerEventType::Cancel
                        ) {
                            if arena.all_members_resolved() && !arena.has_pending_multi_tap() {
                                arena.close();
                            }
                        }

                        events
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            }
        }
    }

    /// Check all arena timers and close resolved arenas
    pub fn tick(&mut self, now: Instant) -> Vec<GestureEvent> {
        let mut all_events = vec![];

        for arena in self.arenas.values_mut() {
            let events = arena.check_timers(now);
            all_events.extend(events);

            // Close arena if all members are resolved (e.g., after timer-based tap recognition)
            if !arena.is_closed() && arena.all_members_resolved() {
                arena.close();
            }
        }

        all_events
    }

    /// Clean up closed arenas
    pub fn cleanup(&mut self) {
        let closed_arenas: Vec<ArenaId> = self
            .arenas
            .iter()
            .filter(|(_, arena)| arena.is_closed())
            .map(|(id, _)| *id)
            .collect();

        for arena_id in closed_arenas {
            if let Some(arena) = self.arenas.remove(&arena_id) {
                self.pointer_to_arena.remove(&arena.pointer_id());
            }
        }
    }

    /// Register a recognizer for a node
    pub fn register_recognizer(&mut self, _node_id: u32, _recognizer: Box<dyn GestureRecognizer>) {
        // This would typically be called when a view is set up
        // For now, recognizers are added when the first pointer down happens
    }

    /// Get all arena IDs
    pub fn arena_ids(&self) -> Vec<ArenaId> {
        self.arenas.keys().copied().collect()
    }

    /// Get debug info
    pub fn debug_info(&self) -> String {
        let mut info = format!("GestureArenaManager: {} arenas\n", self.arenas.len());
        for arena in self.arenas.values() {
            info.push_str(&format!("  {}\n", arena.debug_info()));
        }
        info
    }
}

impl Default for GestureArenaManager {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn test_arena_single_tap() {
        let mut manager = GestureArenaManager::new();
        let node_id = 1;

        // Add tap recognizer
        let down = PointerEventBuilder::new(0)
            .node_id(node_id)
            .down(100.0, 100.0);
        let arena = manager.get_or_create_arena(0, node_id);
        arena.add_member(Box::new(TapGestureRecognizer::single_tap(1, node_id)));

        let events = manager.handle_pointer_event(&down);
        assert!(events.is_empty());

        let up = PointerEventBuilder::new(0)
            .node_id(node_id)
            .up_at(100.0, 100.0);
        let events = manager.handle_pointer_event(&up);
        events.assert_tap(1).assert_count(1);
    }

    #[test]
    fn test_arena_tap_vs_long_press() {
        let mut manager = GestureArenaManager::new();
        let node_id = 1;

        // Add both recognizers
        let down = PointerEventBuilder::new(0)
            .node_id(node_id)
            .down(100.0, 100.0);
        let arena = manager.get_or_create_arena(0, node_id);
        arena.add_member(Box::new(TapGestureRecognizer::single_tap(1, node_id)));
        arena.add_member(Box::new(LongPressGestureRecognizer::new(2, node_id)));

        let events = manager.handle_pointer_event(&down);
        assert!(events.is_empty());

        // Quick tap - should fire immediately
        let up = PointerEventBuilder::new(0)
            .node_id(node_id)
            .up_at(100.0, 100.0);
        let events = manager.handle_pointer_event(&up);
        events.assert_tap(1).assert_count(1);
    }

    #[test]
    fn test_arena_long_press_wins() {
        let mut manager = GestureArenaManager::new();
        let node_id = 1;
        let start = Instant::now();

        // Add both recognizers
        let down = PointerEventBuilder::new(0)
            .node_id(node_id)
            .down(100.0, 100.0);
        let arena = manager.get_or_create_arena(0, node_id);
        arena.add_member(Box::new(TapGestureRecognizer::single_tap(1, node_id)));
        arena.add_member(Box::new(LongPressGestureRecognizer::new(2, node_id)));

        manager.handle_pointer_event(&down);

        // Wait for long press timeout
        let events = manager.tick(start + Duration::from_millis(600));
        events.assert_long_press_start().assert_count(1);
    }

    #[test]
    fn test_arena_pan_wins() {
        let mut manager = GestureArenaManager::new();
        let node_id = 1;

        // Add both recognizers
        let down = PointerEventBuilder::new(0)
            .node_id(node_id)
            .down(100.0, 100.0);
        let arena = manager.get_or_create_arena(0, node_id);
        arena.add_member(Box::new(TapGestureRecognizer::single_tap(1, node_id)));
        arena.add_member(Box::new(PanGestureRecognizer::new(2, node_id)));

        manager.handle_pointer_event(&down);

        // Move beyond slop - pan should win
        let move_evt = PointerEventBuilder::new(0)
            .node_id(node_id)
            .move_to(130.0, 130.0);
        let events = manager.handle_pointer_event(&move_evt);
        events.assert_pan_start().assert_count(1);
    }

    #[test]
    fn test_arena_cleanup() {
        let mut manager = GestureArenaManager::new();
        let node_id = 1;

        // Create arena
        let down = PointerEventBuilder::new(0)
            .node_id(node_id)
            .down(100.0, 100.0);
        let arena = manager.get_or_create_arena(0, node_id);
        arena.add_member(Box::new(TapGestureRecognizer::single_tap(1, node_id)));

        manager.handle_pointer_event(&down);
        assert_eq!(manager.arenas.len(), 1);

        // Complete gesture
        let up = PointerEventBuilder::new(0)
            .node_id(node_id)
            .up_at(100.0, 100.0);
        manager.handle_pointer_event(&up);

        // Cleanup
        manager.cleanup();
        assert_eq!(manager.arenas.len(), 0);
    }

    #[test]
    fn test_arena_multiple_pointers() {
        let mut manager = GestureArenaManager::new();
        let node_id = 1;

        // First pointer
        let down1 = PointerEventBuilder::new(0)
            .node_id(node_id)
            .down(100.0, 100.0);
        let arena1 = manager.get_or_create_arena(0, node_id);
        arena1.add_member(Box::new(TapGestureRecognizer::single_tap(1, node_id)));
        manager.handle_pointer_event(&down1);

        // Second pointer (different pointer_id)
        let down2 = PointerEventBuilder::new(1)
            .node_id(node_id)
            .down(200.0, 200.0);
        let arena2 = manager.get_or_create_arena(1, node_id);
        arena2.add_member(Box::new(TapGestureRecognizer::single_tap(2, node_id)));
        manager.handle_pointer_event(&down2);

        assert_eq!(manager.arenas.len(), 2);

        // Complete both
        let up1 = PointerEventBuilder::new(0)
            .node_id(node_id)
            .up_at(100.0, 100.0);
        let up2 = PointerEventBuilder::new(1)
            .node_id(node_id)
            .up_at(200.0, 200.0);
        manager.handle_pointer_event(&up1);
        manager.handle_pointer_event(&up2);

        manager.cleanup();
        assert_eq!(manager.arenas.len(), 0);
    }
}
