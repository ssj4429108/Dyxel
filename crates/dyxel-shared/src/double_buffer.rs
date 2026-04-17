// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Double-buffered state system for atomic UI updates
//!
//! This module provides a state management system where:
//! - Front buffer: Read-only for RenderThread
//! - Back buffer: Write-only for LogicThread
//! - Staging buffer: Transaction accumulation
//!
//! The swap happens atomically at EndTransaction, ensuring
//! RenderThread never sees a partially updated state.

use crate::state::SharedState;
use std::sync::atomic::{AtomicU64, Ordering};

/// Generation counter for tracking buffer versions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generation(pub u64);

impl Generation {
    pub fn next(&self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// A versioned snapshot of the UI state
pub struct StateSnapshot {
    pub state: SharedState,
    pub generation: Generation,
    /// Monotonically increasing frame sequence
    pub sequence: u64,
}

impl std::fmt::Debug for StateSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateSnapshot")
            .field("generation", &self.generation)
            .field("sequence", &self.sequence)
            .field("node_count", &self.state.nodes.len())
            .finish()
    }
}

impl StateSnapshot {
    pub fn new(sequence: u64) -> Self {
        Self {
            state: SharedState::new(),
            generation: Generation(0),
            sequence,
        }
    }

    pub fn from_state(state: SharedState, generation: Generation, sequence: u64) -> Self {
        Self {
            state,
            generation,
            sequence,
        }
    }
}

/// Double-buffered state container
///
/// Thread safety:
/// - Front buffer: Protected by RwLock, RenderThread acquires read lock
/// - Back buffer: Only accessed by LogicThread (exclusive)
/// - Swap: Atomic pointer exchange + memory barrier
pub struct DoubleBufferedState {
    /// Current front buffer index (0 or 1)
    front_index: AtomicU64,

    /// The two state buffers
    buffers: [StateSnapshot; 2],

    /// Staging area for accumulating transaction commands
    staging: SharedState,

    /// Current sequence number
    sequence: AtomicU64,
}

impl DoubleBufferedState {
    pub fn new() -> Self {
        Self {
            front_index: AtomicU64::new(0),
            buffers: [StateSnapshot::new(0), StateSnapshot::new(0)],
            staging: SharedState::new(),
            sequence: AtomicU64::new(0),
        }
    }

    /// Get immutable reference to front buffer (for RenderThread)
    pub fn front(&self) -> &StateSnapshot {
        let idx = self.front_index.load(Ordering::Acquire) as usize;
        &self.buffers[idx]
    }

    /// Get mutable reference to back buffer (for LogicThread)
    pub fn back_mut(&mut self) -> &mut StateSnapshot {
        let idx = self.back_index() as usize;
        &mut self.buffers[idx]
    }

    /// Get mutable reference to staging buffer
    pub fn staging_mut(&mut self) -> &mut SharedState {
        &mut self.staging
    }

    /// Get immutable reference to staging buffer
    pub fn staging(&self) -> &SharedState {
        &self.staging
    }

    /// Apply staging to back buffer and swap
    ///
    /// This is the core atomic operation:
    /// 1. Copy staging -> back buffer
    /// 2. Increment sequence
    /// 3. Atomic swap front index
    pub fn commit_and_swap(&mut self) -> Generation {
        let back_idx = self.back_index() as usize;
        let front_idx = self.front_index.load(Ordering::Relaxed) as usize;

        // Step 1: Copy staging to back buffer
        // TODO: This is a deep copy - optimize with persistent data structures
        self.buffers[back_idx].state = std::mem::replace(&mut self.staging, SharedState::new());

        // Step 2: Update generation and sequence
        let new_gen = self.buffers[front_idx].generation.next();
        self.buffers[back_idx].generation = new_gen;
        self.buffers[back_idx].sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;

        // Step 3: Atomic swap front index
        self.front_index.store(back_idx as u64, Ordering::Release);

        new_gen
    }

    /// Reset staging buffer (for transaction abort)
    pub fn reset_staging(&mut self) {
        self.staging = SharedState::new();
    }

    /// Check if there are pending changes in staging
    pub fn has_pending_changes(&self) -> bool {
        !self.staging.nodes.is_empty()
    }

    /// Get current sequence number
    pub fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }

    fn back_index(&self) -> u64 {
        let front = self.front_index.load(Ordering::Relaxed);
        1 - front // Toggle between 0 and 1
    }
}

/// Transaction context for batching commands
#[derive(Debug)]
pub struct TransactionContext {
    pub seq_id: u32,
    pub flags: u16,
    pub start_sequence: u64,
    pub commands_applied: u32,
}

impl TransactionContext {
    pub fn new(seq_id: u32, flags: u16, sequence: u64) -> Self {
        Self {
            seq_id,
            flags,
            start_sequence: sequence,
            commands_applied: 0,
        }
    }

    pub fn is_layout_only(&self) -> bool {
        self.flags & 0x01 != 0
    }

    pub fn is_immediate(&self) -> bool {
        self.flags & 0x02 != 0
    }

    pub fn is_mergeable(&self) -> bool {
        self.flags & 0x04 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_double_buffer_swap() {
        let mut dbs = DoubleBufferedState::new();

        // Initially front is buffer 0
        assert_eq!(dbs.front_index.load(Ordering::Relaxed), 0);

        // Add something to staging
        dbs.staging.create_node(0);

        // Commit and swap
        let gen = dbs.commit_and_swap();
        assert_eq!(gen.0, 1);

        // Now front should be buffer 1
        assert_eq!(dbs.front_index.load(Ordering::Relaxed), 1);

        // Front buffer should have the node
        assert!(dbs.front().state.nodes.contains_key(&0));
    }

    #[test]
    fn test_generation_increments() {
        let mut dbs = DoubleBufferedState::new();
        dbs.staging.create_node(0);

        let gen1 = dbs.commit_and_swap();
        dbs.staging.create_node(1);
        let gen2 = dbs.commit_and_swap();

        assert_eq!(gen2.0, gen1.0 + 1);
    }
}
