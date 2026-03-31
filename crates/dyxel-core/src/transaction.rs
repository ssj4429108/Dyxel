// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Re-export for tests and other consumers
pub use dyxel_shared::{OpCode, DirtyField, DirtyTracker};

use std::collections::HashMap;

/// Transaction processing state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    /// No active transaction
    Idle,
    /// Transaction in progress, commands being staged
    Active { seq_id: u32, flags: u16 },
    /// Transaction committed, ready to apply
    Committed { seq_id: u32 },
    /// Transaction aborted
    Aborted { seq_id: u32 },
}

impl Default for TransactionState {
    fn default() -> Self { TransactionState::Idle }
}

/// A staged command waiting to be applied
#[derive(Debug, Clone)]
pub struct StagedCommand {
    pub opcode: OpCode,
    pub node_id: u32,
    pub payload: Vec<u8>,
    pub dirty_fields: DirtyField,
}

/// Command deduplication key: (node_id, field_type)
/// Multiple updates to same node+field are merged into last one
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DedupKey {
    node_id: u32,
    field_type: FieldType,
    seq: u64,  // Sequence number for unique keys (0 for normal ops)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
enum FieldType {
    Color,
    Width,
    Height,
    Position,  // x, y via inset/margin
    // Split layout properties - don't merge different layout attributes
    FlexDirection,
    JustifyContent,
    AlignItems,
    FlexWrap,
    AlignContent,
    FlexGrow,
    Text,      // text content
    Children,  // AddChild - needs special handling
    Other,
}

impl FieldType {
    fn from_opcode(op: &OpCode) -> Self {
        match op {
            OpCode::SetColor | OpCode::SetColorCompact | OpCode::SetTextColor => FieldType::Color,
            OpCode::SetWidth | OpCode::SetWidthCompact => FieldType::Width,
            OpCode::SetHeight | OpCode::SetHeightCompact => FieldType::Height,
            OpCode::SetFlexDirection => FieldType::FlexDirection,
            OpCode::SetJustifyContent => FieldType::JustifyContent,
            OpCode::SetAlignItems => FieldType::AlignItems,
            OpCode::SetFlexWrap => FieldType::FlexWrap,
            OpCode::SetAlignContent => FieldType::AlignContent,
            OpCode::SetFlexGrow => FieldType::FlexGrow,
            OpCode::SetText | OpCode::SetTextContent => FieldType::Text,
            OpCode::AddChild => FieldType::Children,
            _ => FieldType::Other,
        }
    }
}

/// Accumulates and deduplicates commands within a transaction
#[derive(Debug, Default)]
pub struct CommandAccumulator {
    /// Staged commands indexed by dedup key for merging
    commands: HashMap<DedupKey, StagedCommand>,
    /// Original order preservation for final iteration
    order: Vec<DedupKey>,
    /// Sequence counter for unique keys (used for AddChild)
    seq_counter: u64,
}

impl CommandAccumulator {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
            order: Vec::new(),
            seq_counter: 0,
        }
    }
    
    /// Stage a command, merging with existing if same node+field
    pub fn stage(&mut self, cmd: StagedCommand) {
        let field_type = FieldType::from_opcode(&cmd.opcode);
        
        // For AddChild/CreateNode/CreateTextNode, never merge - always create unique key
        let (key, always_push) = match field_type {
            FieldType::Children | FieldType::Other => {
                // These ops should never be merged
                self.seq_counter += 1;
                (DedupKey {
                    node_id: cmd.node_id,
                    field_type,
                    seq: self.seq_counter,
                }, true)
            }
            _ => {
                (DedupKey {
                    node_id: cmd.node_id,
                    field_type,
                    seq: 0,
                }, false)
            }
        };
        
        // Add to order if unique or not yet seen
        if always_push || !self.commands.contains_key(&key) {
            self.order.push(key);
        }
        self.commands.insert(key, cmd);
    }
    
    /// Get commands in original order (after deduplication)
    pub fn get_commands(&self) -> Vec<&StagedCommand> {
        self.order.iter()
            .filter_map(|key| self.commands.get(key))
            .collect()
    }
    
    /// Get mutable commands for processing
    pub fn drain_commands(&mut self) -> Vec<StagedCommand> {
        let mut result = Vec::with_capacity(self.order.len());
        for key in &self.order {
            if let Some(cmd) = self.commands.remove(key) {
                result.push(cmd);
            }
        }
        self.order.clear();
        result
    }
    
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
    
    pub fn clear(&mut self) {
        self.commands.clear();
        self.order.clear();
    }
}

/// Complete transaction processor managing state machine
#[derive(Debug, Default)]
pub struct TransactionProcessor {
    pub state: TransactionState,
    pub accumulator: CommandAccumulator,
    pub dirty_tracker: DirtyTracker,
    /// Pending render flag - set when transaction commits
    pub render_pending: bool,
}

impl TransactionProcessor {
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Start a new transaction
    pub fn begin(&mut self, seq_id: u32, flags: u16) -> Result<(), String> {
        match self.state {
            TransactionState::Idle => {
                self.state = TransactionState::Active { seq_id, flags };
                self.accumulator.clear();
                // Transaction started
                Ok(())
            }
            TransactionState::Active { seq_id: existing, .. } => {
                // Nested transaction not supported, abort current
                Err(format!("Cannot begin transaction {} while {} is active", seq_id, existing))
            }
            _ => {
                // Previous transaction not cleaned up
                self.reset();
                self.state = TransactionState::Active { seq_id, flags };
                Ok(())
            }
        }
    }
    
    /// Stage a command within active transaction
    pub fn stage_command(&mut self, cmd: StagedCommand) -> Result<(), String> {
        match self.state {
            TransactionState::Active { .. } => {
                self.dirty_tracker.mark_dirty(cmd.node_id, cmd.dirty_fields);
                self.accumulator.stage(cmd);
                Ok(())
            }
            _ => Err("No active transaction to stage command".to_string()),
        }
    }
    
    /// Commit the active transaction
    pub fn commit(&mut self, seq_id: u32) -> Result<Vec<StagedCommand>, String> {
        match self.state {
            TransactionState::Active { seq_id: active_id, .. } if active_id == seq_id => {
                self.state = TransactionState::Committed { seq_id };
                self.render_pending = true;
                let commands = self.accumulator.drain_commands();
                // Transaction committed
                Ok(commands)
            }
            TransactionState::Active { seq_id: active_id, .. } => {
                Err(format!("Commit seq_id mismatch: expected {}, got {}", active_id, seq_id))
            }
            _ => Err("No active transaction to commit".to_string()),
        }
    }
    
    /// Abort the active transaction
    pub fn abort(&mut self, seq_id: u32) -> Result<(), String> {
        match self.state {
            TransactionState::Active { seq_id: active_id, .. } if active_id == seq_id => {
                self.state = TransactionState::Aborted { seq_id };
                self.accumulator.clear();
                // Rollback dirty marks for this transaction would require more tracking
                // For now, we keep dirty marks (safe but may cause extra render)
                // Transaction aborted
                Ok(())
            }
            _ => Err("No matching active transaction to abort".to_string()),
        }
    }
    
    /// Reset to idle state
    pub fn reset(&mut self) {
        self.state = TransactionState::Idle;
        self.accumulator.clear();
        self.render_pending = false;
    }
    
    /// Check if render is needed and clear the flag
    pub fn take_render_pending(&mut self) -> bool {
        let pending = self.render_pending;
        self.render_pending = false;
        pending
    }
    
    /// Apply committed commands to shared state
    pub fn apply_commands<F>(&mut self, mut apply_fn: F) 
    where
        F: FnMut(&StagedCommand),
    {
        if let TransactionState::Committed { .. } = self.state {
            for cmd in self.accumulator.drain_commands() {
                apply_fn(&cmd);
            }
            self.state = TransactionState::Idle;
        }
    }
}

/// Helper: Extract node_id from opcode payload for dirty tracking
pub fn extract_node_id(_opcode: &OpCode, payload: &[u8]) -> Option<u32> {
    if payload.len() >= 4 {
        Some(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]))
    } else {
        None
    }
}

/// Helper: Determine dirty fields from opcode
pub fn get_dirty_field_for_opcode(opcode: &OpCode) -> DirtyField {
    match opcode {
        OpCode::CreateNode | OpCode::CreateTextNode => DirtyField::None,
        OpCode::SetColor | OpCode::SetColorCompact | OpCode::SetTextColor => DirtyField::Style,
        OpCode::SetWidth | OpCode::SetWidthCompact | 
        OpCode::SetHeight | OpCode::SetHeightCompact => DirtyField::Size,
        OpCode::SetText | OpCode::SetTextContent => DirtyField::Text,
        OpCode::AddChild => DirtyField::Children,
        OpCode::SetFlexDirection | OpCode::SetJustifyContent | 
        OpCode::SetAlignItems | OpCode::SetFlexWrap | OpCode::SetAlignContent |
        OpCode::SetFlexGrow | OpCode::SetPadding => DirtyField::Layout,
        // === LayoutRegistry Operations (read-only, no dirty) ===
        OpCode::GetLayout | OpCode::IsLayoutDirty | OpCode::ClearLayoutDirty | OpCode::GetLayoutBatch => DirtyField::None,
        // === Transaction Operations (no dirty) ===
        OpCode::BeginTransaction | OpCode::EndTransaction | OpCode::AbortTransaction | OpCode::SetNodeDirty => DirtyField::None,
        _ => DirtyField::Style,
    }
}
