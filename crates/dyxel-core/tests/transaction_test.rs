// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for Transaction system

use dyxel_core::transaction::*;
use dyxel_shared::{OpCode, DirtyField};

#[test]
fn test_transaction_basic() {
    let mut acc = TransactionAccumulator::new();
    
    // Begin transaction
    acc.begin(1, 0);
    assert!(acc.is_active);
    assert_eq!(acc.seq_id, 1);
    
    // Stage some commands
    acc.stage(StagedCommand {
        opcode: OpCode::CreateNode,
        payload: vec![0, 0, 0, 0], // node 0
        node_id: Some(0),
        dirty_fields: 0,
    });
    
    acc.stage(StagedCommand {
        opcode: OpCode::SetColor,
        payload: vec![0, 255, 0, 0], // node 0, red
        node_id: Some(0),
        dirty_fields: DirtyField::Style.bits(),
    });
    
    assert_eq!(acc.command_count(), 2);
    
    // End transaction
    let cmds = acc.end();
    assert!(!acc.is_active);
    assert_eq!(cmds.len(), 2);
}

#[test]
fn test_command_deduplication() {
    let mut acc = TransactionAccumulator::new();
    acc.begin(1, 0);
    
    // Multiple color changes to same node - should deduplicate
    for i in 0..5 {
        acc.stage(StagedCommand {
            opcode: OpCode::SetColor,
            payload: vec![0, i as u8 * 50, 0, 0],
            node_id: Some(0),
            dirty_fields: DirtyField::Style.bits(),
        });
    }
    
    let cmds = acc.end();
    assert_eq!(cmds.len(), 1, "Should deduplicate to single command");
    assert_eq!(cmds[0].payload[1], 200, "Should keep last value"); // 4 * 50 = 200
}

#[test]
fn test_different_fields_no_dedup() {
    let mut acc = TransactionAccumulator::new();
    acc.begin(1, 0);
    
    // Same node, different fields - should NOT deduplicate
    acc.stage(StagedCommand {
        opcode: OpCode::SetColor,
        payload: vec![0, 255, 0, 0],
        node_id: Some(0),
        dirty_fields: DirtyField::Style.bits(),
    });
    
    acc.stage(StagedCommand {
        opcode: OpCode::SetWidth,
        payload: vec![0, 1, 0, 0, 0, 0, 0, 0, 0], // 100px
        node_id: Some(0),
        dirty_fields: DirtyField::Size.bits(),
    });
    
    acc.stage(StagedCommand {
        opcode: OpCode::SetFlexDirection,
        payload: vec![0, 1, 0, 0, 0], // Row
        node_id: Some(0),
        dirty_fields: DirtyField::Layout.bits(),
    });
    
    let cmds = acc.end();
    assert_eq!(cmds.len(), 3, "Different fields should not be deduplicated");
}

#[test]
fn test_transaction_abort() {
    let mut acc = TransactionAccumulator::new();
    acc.begin(1, 0);
    
    acc.stage(StagedCommand {
        opcode: OpCode::CreateNode,
        payload: vec![0, 0, 0, 0],
        node_id: Some(0),
        dirty_fields: 0,
    });
    
    acc.abort();
    
    assert!(!acc.is_active);
    assert!(acc.is_empty());
}

#[test]
fn test_layout_only_optimization() {
    let mut acc = TransactionAccumulator::new();
    acc.begin(1, 0x01); // SkipIfLayoutOnly flag
    
    // Add layout-only commands
    acc.stage(StagedCommand {
        opcode: OpCode::SetWidth,
        payload: vec![0, 1, 0, 0, 0, 0, 0, 0, 0],
        node_id: Some(0),
        dirty_fields: DirtyField::Size.bits(),
    });
    
    acc.stage(StagedCommand {
        opcode: OpCode::SetFlexDirection,
        payload: vec![0, 1, 0, 0, 0],
        node_id: Some(0),
        dirty_fields: DirtyField::Layout.bits(),
    });
    
    assert!(acc.is_layout_only(), "Should detect layout-only transaction");
}

#[test]
fn test_opcode_to_dirty_field() {
    assert_eq!(opcode_to_dirty_field(OpCode::SetColor), DirtyField::Style.bits());
    assert_eq!(opcode_to_dirty_field(OpCode::SetWidth), DirtyField::Size.bits());
    assert_eq!(opcode_to_dirty_field(OpCode::SetFlexDirection), DirtyField::Layout.bits());
    assert_eq!(opcode_to_dirty_field(OpCode::SetText), DirtyField::Text.bits());
    assert_eq!(opcode_to_dirty_field(OpCode::AddChild), DirtyField::Children.bits());
}

#[test]
fn test_extract_node_id() {
    assert_eq!(
        extract_node_id(OpCode::CreateNode, &[5, 0, 0, 0]),
        Some(5)
    );
    assert_eq!(
        extract_node_id(OpCode::SetColorCompact, &[]),
        None // Compact commands need cur_id context
    );
    assert_eq!(
        extract_node_id(OpCode::BeginTransaction, &[1, 0, 0, 0]),
        None // Transaction commands have no node
    );
}

#[test]
fn test_complex_scenario() {
    // Simulate: Create 3 nodes, modify node 0 multiple times, modify node 1 once
    let mut acc = TransactionAccumulator::new();
    acc.begin(1, 0);
    
    // Create nodes
    for i in 0..3 {
        acc.stage(StagedCommand {
            opcode: OpCode::CreateNode,
            payload: vec![i, 0, 0, 0],
            node_id: Some(i as u32),
            dirty_fields: 0,
        });
    }
    
    // Multiple updates to node 0 (should dedup)
    for i in 1..=5 {
        acc.stage(StagedCommand {
            opcode: OpCode::SetColor,
            payload: vec![0, i * 40, 0, 0],
            node_id: Some(0),
            dirty_fields: DirtyField::Style.bits(),
        });
    }
    
    // Width update to node 0 (different field, no dedup)
    acc.stage(StagedCommand {
        opcode: OpCode::SetWidth,
        payload: vec![0, 1, 0, 0, 0, 0, 0, 0, 0], // 100px
        node_id: Some(0),
        dirty_fields: DirtyField::Size.bits(),
    });
    
    // Single update to node 1
    acc.stage(StagedCommand {
        opcode: OpCode::SetColor,
        payload: vec![1, 0, 255, 0],
        node_id: Some(1),
        dirty_fields: DirtyField::Style.bits(),
    });
    
    let cmds = acc.end();
    
    // Expected:
    // - 3 CreateNode
    // - 1 SetColor (for node 0, deduplicated from 5)
    // - 1 SetWidth (for node 0)
    // - 1 SetColor (for node 1)
    // = 6 commands total
    assert_eq!(cmds.len(), 6, "Complex scenario should have 6 commands after dedup");
}
