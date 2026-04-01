// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture Arena
//! 
//! Inspired by Flutter's GestureArena, this module manages competing gesture
//! recognizers and resolves conflicts using a winner-takes-all approach.
//!
//! ## Arena Rules
//! 
//! 1. When a pointer goes down, a new arena is created for that pointer
//! 2. Multiple recognizers can enter the arena
//! 3. Recognizers compete until one claims victory (accepts)
//! 4. Once a winner is declared, all other recognizers are rejected
//! 5. If no winner is declared by pointer up, the arena is swept and a
//!    heuristic winner is chosen

use crate::{
    events::{GestureEvent, PointerEvent, PointerEventType},
    recognizer::{GestureRecognizer, RecognizerState},
    GestureSettings,
    GestureConfig,
    GestureType,
    TapGestureRecognizer,
    LongPressGestureRecognizer,
    PanGestureRecognizer,
};

use std::collections::HashMap;

/// Unique identifier for a gesture arena
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArenaId(u64);

impl ArenaId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// A member of the gesture arena
struct ArenaMember {
    /// The recognizer
    recognizer: Box<dyn GestureRecognizer>,
    /// Whether this recognizer has been accepted
    accepted: bool,
    /// Whether this recognizer has been rejected
    rejected: bool,
}

/// Gesture arena for a single pointer
/// 
/// Manages competing recognizers for one pointer sequence
pub struct GestureArena {
    id: ArenaId,
    pointer_id: u32,
    #[allow(dead_code)]
    target_node_id: u32,
    members: Vec<ArenaMember>,
    /// Whether a winner has been declared
    has_winner: bool,
    /// Whether the arena is open to new members
    is_open: bool,
    /// Tracked pointers (all pointers in this arena)
    tracked_pointers: HashMap<u32, crate::events::PointerData>,
    /// Next member ID
    next_member_id: u32,
}

impl GestureArena {
    pub fn new(id: ArenaId, pointer_id: u32, target_node_id: u32) -> Self {
        Self {
            id,
            pointer_id,
            target_node_id,
            members: Vec::new(),
            has_winner: false,
            is_open: true,
            tracked_pointers: HashMap::new(),
            next_member_id: 0,
        }
    }

    /// Add a recognizer to the arena
    /// 
    /// Returns the member ID. Only allowed while arena is open.
    pub fn add_recognizer(&mut self, recognizer: Box<dyn GestureRecognizer>) -> u32 {
        if !self.is_open {
            return 0;
        }

        let member_id = self.next_member_id;
        self.next_member_id += 1;

        self.members.push(ArenaMember {
            recognizer,
            accepted: false,
            rejected: false,
        });

        member_id
    }

    /// Close the arena to new members
    /// 
    /// This is called when all recognizers have had a chance to enter
    pub fn close(&mut self) {
        self.is_open = false;
    }

    /// Handle a pointer event
    /// 
    /// Routes the event to all members and manages the arena state
    pub fn handle_event(&mut self, event: PointerEvent) -> Vec<GestureEvent> {
        let mut all_events = Vec::new();
        
        // Track pointer data
        match event.event_type {
            PointerEventType::Down => {
                let data = crate::events::PointerData::new(&event);
                self.tracked_pointers.insert(event.pointer_id, data);
            }
            PointerEventType::Move => {
                if let Some(data) = self.tracked_pointers.get_mut(&event.pointer_id) {
                    data.update(&event);
                }
            }
            PointerEventType::Up | PointerEventType::Cancel => {
                // Don't remove from tracked_pointers yet - recognizers may need it
                // We'll clear it after processing
            }
        }

        // Route to all members
        let mut winner_to_declare: Option<u32> = None;
        

        
        for (_idx, member) in &mut self.members.iter_mut().enumerate() {
            // Skip rejected members
            if member.rejected {
                continue;
            }
            
            // For accepted members, only send Move events (for Pan/Drag continuation)
            // But allow Up/Cancel events for gestures that need them (e.g., LongPress)
            if member.accepted && !matches!(event.event_type, 
                PointerEventType::Move | PointerEventType::Up | PointerEventType::Cancel) {
                continue;
            }

            // Handle event
            let events = member.recognizer.handle_event(&event, &self.tracked_pointers);

            all_events.extend(events);

            // Check if this recognizer has claimed victory
            // In Flutter, recognizers call acceptGesture() to win
            // We map this to Accepted state
            match member.recognizer.state() {
                RecognizerState::Accepted => {
                    // Only declare winner immediately for non-tap gestures
                    // Tap gestures use Changed state and wait for sweep to allow 
                    // multi-tap competition (e.g., SingleTap vs DoubleTap)
                    member.accepted = true;
                    winner_to_declare = Some(member.recognizer.target_node_id());
                }
                RecognizerState::Changed => {
                    // LongPress and Pan move to Changed when active
                    // They should win the arena at this point
                    member.accepted = true;
                    winner_to_declare = Some(member.recognizer.target_node_id());
                }
                RecognizerState::Rejected => {
                    member.rejected = true;
                }
                RecognizerState::Cancelled => {
                    member.rejected = true;
                }
                _ => {}
            }
        }
        
        // Declare winner outside the borrow
        if let Some(winner_id) = winner_to_declare {
            self.declare_winner(winner_id);
        }

        // Check if arena should be swept (on pointer up)
        if matches!(event.event_type, PointerEventType::Up) && !self.has_winner {
            self.sweep();
        }

        // Clean up resolved members' events if we have a winner
        if self.has_winner {
            all_events.retain(|e| {
                // Only keep events from accepted recognizers
                self.members.iter().any(|m| {
                    m.accepted && m.recognizer.target_node_id() == e.target_node_id
                })
            });
        }

        // Clean up tracked_pointers for Up/Cancel events after processing
        if matches!(event.event_type, PointerEventType::Up | PointerEventType::Cancel) {
            self.tracked_pointers.remove(&event.pointer_id);
        }

        all_events
    }

    /// Declare a winner for this arena
    /// 
    /// All other recognizers will be rejected
    fn declare_winner(&mut self, winner_node_id: u32) {
        if self.has_winner {
            return;
        }

        self.has_winner = true;

        for member in &mut self.members {
            if member.recognizer.target_node_id() != winner_node_id {
                member.rejected = true;
                member.recognizer.reject();
            } else {
                member.accepted = true;
                member.recognizer.accept();
            }
        }

    }

    /// Sweep the arena to select a winner
    /// 
    /// Called when pointer goes up and no winner has been declared.
    /// Uses heuristics to choose the best candidate.
    fn sweep(&mut self) {
        if self.has_winner || self.members.is_empty() {
            return;
        }

        // Find the most eager recognizer (one that has made most progress)
        // Priority: Pan > LongPress > Tap
        // Skip recognizers that are waiting for multi-tap (e.g., DoubleTap after first tap)
        let mut best_candidate: Option<u32> = None;
        let mut best_priority = 0;

        for member in &self.members {
            if member.rejected {
                continue;
            }

            // Skip tap recognizers waiting for multi-tap
            if let Some(tap_recognizer) = member.recognizer.as_any().downcast_ref::<TapGestureRecognizer>() {
                if tap_recognizer.is_waiting_for_multi_tap() {
                    continue;
                }
            }

            let priority = match member.recognizer.state() {
                // Pan recognizers have highest priority when active
                RecognizerState::Changed => 100,
                // Then those that have begun
                RecognizerState::Began => 50,
                // Then possible ones
                RecognizerState::Possible => 10,
                _ => 0,
            };

            if priority > best_priority {
                best_priority = priority;
                best_candidate = Some(member.recognizer.target_node_id());
            }
        }

        if let Some(winner) = best_candidate {
            self.declare_winner(winner);
        } else {
            // No viable candidate found
            // Check if all remaining candidates are waiting for multi-tap
            // If so, don't reject them - just leave arena open for next tap
            let all_waiting_for_multi_tap = self.members.iter().all(|m| {
                if m.rejected {
                    return true; // Already rejected, consider as "not blocking"
                }
                if let Some(tap) = m.recognizer.as_any().downcast_ref::<TapGestureRecognizer>() {
                    tap.is_waiting_for_multi_tap()
                } else {
                    false
                }
            });
            
            if !all_waiting_for_multi_tap {
                // No viable candidate and not all waiting for multi-tap, reject all
                for member in &mut self.members {
                    if !member.rejected {
                        member.rejected = true;
                        member.recognizer.reject();
                    }
                }
            }
        }
    }

    /// Cancel all recognizers in the arena
    pub fn cancel_all(&mut self) {
        for member in &mut self.members {
            if !member.accepted && !member.rejected {
                member.recognizer.cancel();
                member.rejected = true;
            }
        }
    }

    /// Whether the arena is resolved (has a winner or all rejected)
    pub fn is_resolved(&self) -> bool {
        self.has_winner || self.members.iter().all(|m| m.accepted || m.rejected)
    }

    /// Whether the arena is empty
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn id(&self) -> ArenaId {
        self.id
    }

    pub fn pointer_id(&self) -> u32 {
        self.pointer_id
    }

    pub fn target_node_id(&self) -> u32 {
        self.target_node_id
    }

    /// Check if any member recognizer is waiting for multi-tap
    /// This is used to determine if the arena should be kept in delayed state
    pub fn is_waiting_for_multi_tap(&self) -> bool {
        self.members.iter().any(|member| {
            // Check if this is a tap recognizer waiting for more taps
            if let Some(tap_recognizer) = member.recognizer.as_any().downcast_ref::<TapGestureRecognizer>() {
                tap_recognizer.is_waiting_for_multi_tap()
            } else {
                false
            }
        })
    }
}

/// Manages multiple gesture arenas
/// 
/// One arena per active pointer. Supports delayed arena reuse for multi-tap gestures.
pub struct GestureArenaManager {
    /// Active arenas by ID
    arenas: HashMap<ArenaId, GestureArena>,
    /// Map from pointer ID to arena ID
    pointer_to_arena: HashMap<u32, ArenaId>,
    /// Delayed arenas waiting for multi-tap, keyed by (pointer_id, node_id)
    /// These arenas are kept alive after pointer up to allow double/triple tap detection
    delayed_arenas: HashMap<(u32, u32), (ArenaId, std::time::Instant)>,
    /// Next arena ID
    next_arena_id: u64,
}

impl GestureArenaManager {
    pub fn new() -> Self {
        Self {
            arenas: HashMap::new(),
            pointer_to_arena: HashMap::new(),
            delayed_arenas: HashMap::new(),
            next_arena_id: 1,
        }
    }

    /// Clean up expired delayed arenas
    fn cleanup_expired_delayed_arenas(&mut self, timeout_ms: u64) {
        let now = std::time::Instant::now();
        let expired: Vec<_> = self
            .delayed_arenas
            .iter()
            .filter(|(_, (_, created_at))| {
                now.duration_since(*created_at).as_millis() as u64 > timeout_ms
            })
            .map(|(key, _)| *key)
            .collect();
        
        for key in expired {
            if let Some((arena_id, _)) = self.delayed_arenas.remove(&key) {
                self.arenas.remove(&arena_id);
            }
        }
    }

    /// Create a new arena for a pointer with specific gesture types
    /// 
    /// Only creates recognizers for the specified gesture types.
    /// If gesture_types is empty, no recognizers will be created.
    /// 
    /// For multi-tap gestures, this may reuse a delayed arena from a previous tap.
    pub fn create_arena(
        &mut self,
        pointer_id: u32,
        target_node_id: u32,
        settings: GestureSettings,
        gesture_types: &[GestureType],
    ) -> ArenaId {
        // Clean up expired delayed arenas first
        self.cleanup_expired_delayed_arenas(settings.double_tap_timeout_ms + 100);

        // Check if there's a delayed arena for this (pointer_id, node_id) that we can reuse
        let delayed_key = (pointer_id, target_node_id);
        if let Some((arena_id, _)) = self.delayed_arenas.remove(&delayed_key) {
            if self.arenas.contains_key(&arena_id) {
                // Reactivate the arena
                self.pointer_to_arena.insert(pointer_id, arena_id);
                return arena_id;
            }
        }

        let arena_id = ArenaId::new(self.next_arena_id);
        self.next_arena_id += 1;

        let mut arena = GestureArena::new(arena_id, pointer_id, target_node_id);

        // Add recognizers only for the specified gesture types
        for gesture_type in gesture_types {
            let gesture_config = GestureConfig {
                settings,
                target_node_id,
            };

            let recognizer: Box<dyn GestureRecognizer> = match gesture_type {
                GestureType::Tap => {
                    Box::new(TapGestureRecognizer::new(gesture_config, 1))
                }
                GestureType::DoubleTap => {
                    Box::new(TapGestureRecognizer::new(gesture_config, 2))
                }
                GestureType::LongPress => {
                    Box::new(LongPressGestureRecognizer::new(gesture_config))
                }
                GestureType::Pan => {
                    Box::new(PanGestureRecognizer::new(gesture_config))
                }
            };

            arena.add_recognizer(recognizer);
        }

        // Close arena after adding recognizers
        if !gesture_types.is_empty() {
            arena.close();
        }

        self.arenas.insert(arena_id, arena);
        self.pointer_to_arena.insert(pointer_id, arena_id);

        arena_id
    }

    /// Handle a pointer event in the appropriate arena
    /// 
    /// Returns gesture events to be dispatched
    pub fn handle_pointer_event(
        &mut self,
        arena_id: ArenaId,
        event: PointerEvent,
    ) -> Option<Vec<GestureEvent>> {
        if let Some(arena) = self.arenas.get_mut(&arena_id) {
            let events = arena.handle_event(event);
            
            // After pointer up, check if arena should be closed or moved to delayed state
            if matches!(event.event_type, PointerEventType::Up | PointerEventType::Cancel) {
                // Check if any recognizer is waiting for multi-tap
                let is_waiting_for_multi_tap = arena.is_waiting_for_multi_tap();
                
                if is_waiting_for_multi_tap && matches!(event.event_type, PointerEventType::Up) {
                    // Move arena to delayed state for potential multi-tap continuation
                    let key = (arena.pointer_id, arena.target_node_id);
                    self.delayed_arenas.insert(key, (arena_id, std::time::Instant::now()));
                    self.pointer_to_arena.remove(&arena.pointer_id);
                } else if arena.is_resolved() {
                    // No multi-tap pending and arena resolved, close the arena
                    self.close_arena(arena_id);
                }
            }
            
            Some(events)
        } else {
            None
        }
    }

    /// Close and remove an arena
    pub fn close_arena(&mut self, arena_id: ArenaId) {
        if let Some(arena) = self.arenas.remove(&arena_id) {
            self.pointer_to_arena.remove(&arena.pointer_id);
        }
    }

    /// Get arena ID for a pointer
    pub fn get_arena_for_pointer(&self, pointer_id: u32) -> Option<ArenaId> {
        self.pointer_to_arena.get(&pointer_id).copied()
    }

    /// Cancel all arenas
    pub fn cancel_all(&mut self) {
        for (_, arena) in &mut self.arenas {
            arena.cancel_all();
        }
        self.arenas.clear();
        self.pointer_to_arena.clear();
    }
}

impl Default for GestureArenaManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{PointerEventType};

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

    #[test]
    fn test_arena_creation() {
        let mut manager = GestureArenaManager::new();
        let settings = GestureSettings::default();
        let gestures = vec![GestureType::Tap];
        
        let arena_id = manager.create_arena(0, 1, settings, &gestures);
        
        assert_eq!(manager.get_arena_for_pointer(0), Some(arena_id));
    }

    #[test]
    fn test_tap_wins_over_long_press() {
        let mut manager = GestureArenaManager::new();
        let settings = GestureSettings::default();
        let gestures = vec![GestureType::Tap, GestureType::LongPress];
        
        let arena_id = manager.create_arena(0, 1, settings, &gestures);
        
        // Quick tap (before long press timeout)
        let down = create_pointer_event(PointerEventType::Down, 100.0, 100.0, 0);
        let events = manager.handle_pointer_event(arena_id, down);
        assert!(events.is_none() || events.as_ref().map(|e| e.is_empty()).unwrap_or(true));
        
        // Quick up (tap)
        let up = create_pointer_event(PointerEventType::Up, 100.0, 100.0, 100_000);
        let events = manager.handle_pointer_event(arena_id, up).unwrap();
        
        // Should get tap event
        assert!(!events.is_empty());
        assert!(matches!(events[0].event_type, crate::events::GestureEventType::Tap));
    }

    #[test]
    fn test_pan_wins_with_movement() {
        let mut manager = GestureArenaManager::new();
        let settings = GestureSettings::default();
        let gestures = vec![GestureType::Tap, GestureType::Pan];
        
        let arena_id = manager.create_arena(0, 1, settings, &gestures);
        
        // Down
        let down = create_pointer_event(PointerEventType::Down, 100.0, 100.0, 0);
        manager.handle_pointer_event(arena_id, down);
        
        // Large move (past slop)
        let move_event = create_pointer_event(PointerEventType::Move, 150.0, 150.0, 32_000);
        let events = manager.handle_pointer_event(arena_id, move_event).unwrap();
        
        // Pan should win and generate events
        assert!(!events.is_empty());
    }

    #[test]
    fn test_arena_cleanup() {
        let mut manager = GestureArenaManager::new();
        let settings = GestureSettings::default();
        let gestures = vec![GestureType::Tap];
        
        let arena_id = manager.create_arena(0, 1, settings, &gestures);
        
        // Complete gesture
        let down = create_pointer_event(PointerEventType::Down, 100.0, 100.0, 0);
        manager.handle_pointer_event(arena_id, down);
        
        let up = create_pointer_event(PointerEventType::Up, 100.0, 100.0, 100_000);
        manager.handle_pointer_event(arena_id, up);
        
        // Arena should be cleaned up
        assert!(manager.get_arena_for_pointer(0).is_none());
    }
}
