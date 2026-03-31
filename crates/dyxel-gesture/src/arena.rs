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
            log::warn!("Cannot add recognizer to closed arena {:?}", self.id);
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
                self.tracked_pointers.remove(&event.pointer_id);
            }
        }

        // Route to all members
        let mut winner_to_declare: Option<u32> = None;
        
        for member in &mut self.members {
            // Skip already resolved members
            if member.accepted || member.rejected {
                continue;
            }

            // Handle event
            let events = member.recognizer.handle_event(&event, &self.tracked_pointers);
            all_events.extend(events);

            // Check if this recognizer has claimed victory
            // In Flutter, recognizers call acceptGesture() to win
            // We map this to Accepted state, but Changed also indicates active recognition
            match member.recognizer.state() {
                RecognizerState::Accepted => {
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

        log::debug!(
            "Arena {:?} winner declared: node {}",
            self.id,
            winner_node_id
        );
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
        let mut best_candidate: Option<u32> = None;
        let mut best_priority = 0;

        for member in &self.members {
            if member.rejected {
                continue;
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
            // No viable candidate, reject all
            for member in &mut self.members {
                member.rejected = true;
                member.recognizer.reject();
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
}

/// Manages multiple gesture arenas
/// 
/// One arena per active pointer
pub struct GestureArenaManager {
    /// Active arenas by ID
    arenas: HashMap<ArenaId, GestureArena>,
    /// Map from pointer ID to arena ID
    pointer_to_arena: HashMap<u32, ArenaId>,
    /// Next arena ID
    next_arena_id: u64,
    /// Default recognizers to add to each arena
    default_recognizers: Vec<DefaultRecognizerConfig>,
}

#[derive(Clone)]
struct DefaultRecognizerConfig {
    recognizer_type: DefaultRecognizerType,
    max_taps: u32, // For tap recognizer
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum DefaultRecognizerType {
    Tap,
    DoubleTap,
    TripleTap,
    LongPress,
    Pan,
}

impl GestureArenaManager {
    pub fn new() -> Self {
        let mut manager = Self {
            arenas: HashMap::new(),
            pointer_to_arena: HashMap::new(),
            next_arena_id: 1,
            default_recognizers: Vec::new(),
        };

        // Add default recognizers
        manager.default_recognizers.push(DefaultRecognizerConfig {
            recognizer_type: DefaultRecognizerType::Tap,
            max_taps: 1,
        });
        manager.default_recognizers.push(DefaultRecognizerConfig {
            recognizer_type: DefaultRecognizerType::LongPress,
            max_taps: 0,
        });
        manager.default_recognizers.push(DefaultRecognizerConfig {
            recognizer_type: DefaultRecognizerType::Pan,
            max_taps: 0,
        });

        manager
    }

    /// Create a new arena for a pointer
    /// 
    /// Returns the arena ID
    pub fn create_arena(
        &mut self,
        pointer_id: u32,
        target_node_id: u32,
        settings: GestureSettings,
    ) -> ArenaId {
        let arena_id = ArenaId::new(self.next_arena_id);
        self.next_arena_id += 1;

        let mut arena = GestureArena::new(arena_id, pointer_id, target_node_id);

        // Add default recognizers
        for config in &self.default_recognizers {
            let gesture_config = GestureConfig {
                settings,
                target_node_id,
            };

            let recognizer: Box<dyn GestureRecognizer> = match config.recognizer_type {
                DefaultRecognizerType::Tap => {
                    Box::new(TapGestureRecognizer::new(gesture_config, config.max_taps))
                }
                DefaultRecognizerType::DoubleTap => {
                    Box::new(TapGestureRecognizer::new(gesture_config, 2))
                }
                DefaultRecognizerType::TripleTap => {
                    Box::new(TapGestureRecognizer::new(gesture_config, 3))
                }
                DefaultRecognizerType::LongPress => {
                    Box::new(LongPressGestureRecognizer::new(gesture_config))
                }
                DefaultRecognizerType::Pan => {
                    Box::new(PanGestureRecognizer::new(gesture_config))
                }
            };

            arena.add_recognizer(recognizer);
        }

        // Close arena after adding defaults
        arena.close();

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
            
            // Clean up if arena is resolved
            if arena.is_resolved() {
                self.close_arena(arena_id);
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
        
        let arena_id = manager.create_arena(0, 1, settings);
        
        assert_eq!(manager.get_arena_for_pointer(0), Some(arena_id));
    }

    #[test]
    fn test_tap_wins_over_long_press() {
        let mut manager = GestureArenaManager::new();
        let settings = GestureSettings::default();
        
        let arena_id = manager.create_arena(0, 1, settings);
        
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
        
        let arena_id = manager.create_arena(0, 1, settings);
        
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
        
        let arena_id = manager.create_arena(0, 1, settings);
        
        // Complete gesture
        let down = create_pointer_event(PointerEventType::Down, 100.0, 100.0, 0);
        manager.handle_pointer_event(arena_id, down);
        
        let up = create_pointer_event(PointerEventType::Up, 100.0, 100.0, 100_000);
        manager.handle_pointer_event(arena_id, up);
        
        // Arena should be cleaned up
        assert!(manager.get_arena_for_pointer(0).is_none());
    }
}
