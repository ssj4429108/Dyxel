// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for Transaction system
//!
//! Acceptance Criteria Validation:
//! 1. 同一帧内多次修改同一节点属性，只渲染最后一次
//! 2. 不会出现"背景变了Position没变"tearing phenomenon

use dyxel_core::transaction::*;

/// Helper function to check if dirty fields contain a specific field
fn has_field(fields: u8, field: DirtyField) -> bool {
    fields & field.bits() != 0
}

#[test]
fn test_transaction_basic() {
    let mut tx = TransactionProcessor::new();

    // Begin transaction
    tx.begin(1, 0).unwrap();
    assert!(matches!(
        tx.state,
        TransactionState::Active { seq_id: 1, .. }
    ));

    // Stage some commands
    tx.stage_command(StagedCommand {
        opcode: OpCode::CreateNode,
        node_id: 0,
        payload: vec![0, 0, 0, 0], // node 0
        dirty_fields: DirtyField::None,
    })
    .unwrap();

    tx.stage_command(StagedCommand {
        opcode: OpCode::SetColor,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 255, 0, 0], // node 0, red
        dirty_fields: DirtyField::Style,
    })
    .unwrap();

    // Check commands are staged
    assert!(!tx.accumulator.is_empty());

    // Commit transaction
    let cmds = tx.commit(1).unwrap();
    assert!(matches!(
        tx.state,
        TransactionState::Committed { seq_id: 1 }
    ));
    assert_eq!(cmds.len(), 2);
}

#[test]
fn test_command_deduplication() {
    let mut tx = TransactionProcessor::new();
    tx.begin(1, 0).unwrap();

    // Multiple color changes to same node - should deduplicate
    for i in 0..5 {
        tx.stage_command(StagedCommand {
            opcode: OpCode::SetColor,
            node_id: 0,
            payload: vec![0, 0, 0, 0, i * 50, 0, 0],
            dirty_fields: DirtyField::Style,
        })
        .unwrap();
    }

    let cmds = tx.commit(1).unwrap();
    assert_eq!(cmds.len(), 1, "Should deduplicate to single command");
    assert_eq!(cmds[0].payload[4], 200, "Should keep last value"); // 4 * 50 = 200
}

#[test]
fn test_different_fields_no_dedup() {
    let mut tx = TransactionProcessor::new();
    tx.begin(1, 0).unwrap();

    // Same node, different fields - should NOT deduplicate
    tx.stage_command(StagedCommand {
        opcode: OpCode::SetColor,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 255, 0, 0],
        dirty_fields: DirtyField::Style,
    })
    .unwrap();

    tx.stage_command(StagedCommand {
        opcode: OpCode::SetWidth,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 1, 0, 0, 0, 0], // 100px
        dirty_fields: DirtyField::Size,
    })
    .unwrap();

    tx.stage_command(StagedCommand {
        opcode: OpCode::SetFlexDirection,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 1, 0, 0, 0], // Row
        dirty_fields: DirtyField::Layout,
    })
    .unwrap();

    let cmds = tx.commit(1).unwrap();
    assert_eq!(cmds.len(), 3, "Different fields should not be deduplicated");
}

#[test]
fn test_transaction_abort() {
    let mut tx = TransactionProcessor::new();
    tx.begin(1, 0).unwrap();

    tx.stage_command(StagedCommand {
        opcode: OpCode::CreateNode,
        node_id: 0,
        payload: vec![0, 0, 0, 0],
        dirty_fields: DirtyField::None,
    })
    .unwrap();

    tx.abort(1).unwrap();

    assert!(matches!(tx.state, TransactionState::Aborted { seq_id: 1 }));
    assert!(tx.accumulator.is_empty());
}

#[test]
fn test_dirty_tracker() {
    let mut tracker = DirtyTracker::new();

    // Mark some nodes dirty
    tracker.mark_dirty(0, DirtyField::Style);
    tracker.mark_dirty(1, DirtyField::Size);
    tracker.mark_dirty(5, DirtyField::Layout);

    // Check dirty status
    assert!(tracker.has_dirty());
    assert!(tracker.is_node_dirty(0));
    assert!(tracker.is_node_dirty(1));
    assert!(tracker.is_node_dirty(5));
    assert!(!tracker.is_node_dirty(2));

    // Check dirty fields using helper
    assert!(has_field(tracker.get_dirty_fields(0), DirtyField::Style));
    assert!(has_field(tracker.get_dirty_fields(1), DirtyField::Size));

    // Test iterator
    let dirty_nodes: Vec<u32> = tracker.iter_dirty_nodes().collect();
    assert_eq!(dirty_nodes.len(), 3);
    assert!(dirty_nodes.contains(&0));
    assert!(dirty_nodes.contains(&1));
    assert!(dirty_nodes.contains(&5));

    // Clear and verify
    tracker.clear();
    assert!(!tracker.has_dirty());
    assert!(!tracker.is_node_dirty(0));
}

#[test]
fn test_dirty_field_combinations() {
    //! Test that dirty fields can be combined (e.g., Style | Size)
    let mut tracker = DirtyTracker::new();

    // Mark Style first
    tracker.mark_dirty(0, DirtyField::Style);
    assert!(has_field(tracker.get_dirty_fields(0), DirtyField::Style));
    assert!(!has_field(tracker.get_dirty_fields(0), DirtyField::Size));

    // Mark Size - should combine with existing Style
    tracker.mark_dirty(0, DirtyField::Size);
    let fields = tracker.get_dirty_fields(0);
    assert!(
        has_field(fields, DirtyField::Style),
        "Style should still be set"
    );
    assert!(
        has_field(fields, DirtyField::Size),
        "Size should now be set"
    );

    // Mark Layout - should combine with existing fields
    tracker.mark_dirty(0, DirtyField::Layout);
    let fields = tracker.get_dirty_fields(0);
    assert!(
        has_field(fields, DirtyField::Style),
        "Style should still be set"
    );
    assert!(
        has_field(fields, DirtyField::Size),
        "Size should still be set"
    );
    assert!(
        has_field(fields, DirtyField::Layout),
        "Layout should now be set"
    );
}

#[test]
fn test_get_dirty_field_for_opcode() {
    assert_eq!(
        get_dirty_field_for_opcode(&OpCode::SetColor),
        DirtyField::Style
    );
    assert_eq!(
        get_dirty_field_for_opcode(&OpCode::SetWidth),
        DirtyField::Size
    );
    assert_eq!(
        get_dirty_field_for_opcode(&OpCode::SetFlexDirection),
        DirtyField::Layout
    );
    assert_eq!(
        get_dirty_field_for_opcode(&OpCode::SetText),
        DirtyField::Text
    );
    assert_eq!(
        get_dirty_field_for_opcode(&OpCode::AddChild),
        DirtyField::Children
    );
}

#[test]
fn test_extract_node_id() {
    assert_eq!(extract_node_id(&OpCode::CreateNode, &[5, 0, 0, 0]), Some(5));
    assert_eq!(
        extract_node_id(&OpCode::SetColorCompact, &[]),
        None // Compact commands need cur_id context
    );
}

#[test]
fn test_complex_scenario() {
    // Simulate: Create 3 nodes, modify node 0 multiple times, modify node 1 once
    let mut tx = TransactionProcessor::new();
    tx.begin(1, 0).unwrap();

    // Create nodes (these should NOT be deduplicated)
    for i in 0..3 {
        tx.stage_command(StagedCommand {
            opcode: OpCode::CreateNode,
            node_id: i,
            payload: vec![i as u8, 0, 0, 0],
            dirty_fields: DirtyField::None,
        })
        .unwrap();
    }

    // Multiple updates to node 0 (should dedup - same field)
    for i in 1..=5 {
        tx.stage_command(StagedCommand {
            opcode: OpCode::SetColor,
            node_id: 0,
            payload: vec![0, 0, 0, 0, i * 40, 0, 0],
            dirty_fields: DirtyField::Style,
        })
        .unwrap();
    }

    // Width update to node 0 (different field, no dedup)
    tx.stage_command(StagedCommand {
        opcode: OpCode::SetWidth,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 1, 0, 0, 0, 0], // 100px
        dirty_fields: DirtyField::Size,
    })
    .unwrap();

    // Single update to node 1
    tx.stage_command(StagedCommand {
        opcode: OpCode::SetColor,
        node_id: 1,
        payload: vec![1, 0, 0, 0, 0, 255, 0],
        dirty_fields: DirtyField::Style,
    })
    .unwrap();

    let cmds = tx.commit(1).unwrap();

    // Expected:
    // - 3 CreateNode (not deduplicated - each has unique seq)
    // - 1 SetColor (for node 0, deduplicated from 5)
    // - 1 SetWidth (for node 0)
    // - 1 SetColor (for node 1)
    // = 6 commands total
    assert_eq!(
        cmds.len(),
        6,
        "Complex scenario should have 6 commands after dedup"
    );
}

// ============== 验收标准测试 ==============

#[test]
fn test_same_frame_multiple_updates_renders_last() {
    //! Key test: Multiple modifications to same node in one frame, only render last
    let mut tx = TransactionProcessor::new();
    tx.begin(1, 0).unwrap();

    // Simulate rapid color changes in same frame
    let colors = [
        (255, 0, 0),     // Red
        (0, 255, 0),     // Green
        (0, 0, 255),     // Blue
        (255, 255, 0),   // Yellow
        (128, 128, 128), // Gray (final)
    ];

    for (i, (r, g, b)) in colors.iter().enumerate() {
        tx.stage_command(StagedCommand {
            opcode: OpCode::SetColor,
            node_id: 0,
            payload: vec![0, 0, 0, 0, *r, *g, *b],
            dirty_fields: DirtyField::Style,
        })
        .unwrap();

        // Also update width multiple times
        tx.stage_command(StagedCommand {
            opcode: OpCode::SetWidth,
            node_id: 0,
            payload: vec![0, 0, 0, 0, 1, (i * 10) as u8, 0, 0, 0],
            dirty_fields: DirtyField::Size,
        })
        .unwrap();
    }

    let cmds = tx.commit(1).unwrap();

    // Should have exactly 2 commands: 1 SetColor + 1 SetWidth (both deduplicated)
    assert_eq!(
        cmds.len(),
        2,
        "Same node+field should deduplicate to single command per field"
    );

    // Find the color command and verify it has the LAST value
    let color_cmd = cmds.iter().find(|c| c.opcode == OpCode::SetColor).unwrap();
    assert_eq!(
        color_cmd.payload[4], 128,
        "Should keep last color value (gray R)"
    );
    assert_eq!(
        color_cmd.payload[5], 128,
        "Should keep last color value (gray G)"
    );
    assert_eq!(
        color_cmd.payload[6], 128,
        "Should keep last color value (gray B)"
    );

    // Find the width command and verify it has the LAST value (40)
    let width_cmd = cmds.iter().find(|c| c.opcode == OpCode::SetWidth).unwrap();
    assert_eq!(
        width_cmd.payload[5], 40,
        "Should keep last width value (4 * 10 = 40)"
    );
}

#[test]
fn test_no_tearing_background_and_position_sync() {
    //! 关键测试: 不会出现"背景变了Position没变"tearing phenomenon
    // This is ensured by:
    // 1. All commands in a transaction are applied atomically at commit time
    // 2. Layout and style changes are batched together

    let mut tx = TransactionProcessor::new();
    tx.begin(1, 0).unwrap();

    // Simulate updating both background color and layout properties
    // (which would affect position) in the same transaction

    // First: Set background color
    tx.stage_command(StagedCommand {
        opcode: OpCode::SetColor,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 255, 0, 0], // Red background
        dirty_fields: DirtyField::Style,
    })
    .unwrap();

    // Then: Set width/height (affects layout and position)
    tx.stage_command(StagedCommand {
        opcode: OpCode::SetWidth,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 1, 0x64, 0, 0, 0], // 100px
        dirty_fields: DirtyField::Size,
    })
    .unwrap();

    tx.stage_command(StagedCommand {
        opcode: OpCode::SetHeight,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 1, 0x64, 0, 0, 0], // 100px
        dirty_fields: DirtyField::Size,
    })
    .unwrap();

    // Update flex direction (affects children layout)
    tx.stage_command(StagedCommand {
        opcode: OpCode::SetFlexDirection,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 1, 0, 0, 0], // Row
        dirty_fields: DirtyField::Layout,
    })
    .unwrap();

    let cmds = tx.commit(1).unwrap();

    // All 4 commands should be present (different fields)
    assert_eq!(cmds.len(), 4, "Different fields should all be preserved");

    // Verify dirty tracker has all the right fields marked (using u8 bits)
    assert!(tx.dirty_tracker.is_node_dirty(0));
    let fields = tx.dirty_tracker.get_dirty_fields(0);
    assert!(
        has_field(fields, DirtyField::Style),
        "Style should be dirty"
    );
    assert!(has_field(fields, DirtyField::Size), "Size should be dirty");
    assert!(
        has_field(fields, DirtyField::Layout),
        "Layout should be dirty"
    );

    // The key assurance: all changes are committed together in one batch
    // There's no intermediate state where background is updated but position isn't
}

#[test]
fn test_add_child_never_deduplicated() {
    //! 关键测试: AddChild 操作不应该被合并
    let mut tx = TransactionProcessor::new();
    tx.begin(1, 0).unwrap();

    // Add multiple children to same parent
    for i in 1..=5 {
        tx.stage_command(StagedCommand {
            opcode: OpCode::AddChild,
            node_id: 0,                                  // parent
            payload: vec![0, 0, 0, 0, i as u8, 0, 0, 0], // child id
            dirty_fields: DirtyField::Children,
        })
        .unwrap();
    }

    let cmds = tx.commit(1).unwrap();

    // All 5 AddChild commands should be preserved (never merged)
    assert_eq!(
        cmds.len(),
        5,
        "AddChild commands should never be deduplicated"
    );
}

#[test]
fn test_render_pending_flag() {
    //! 测试渲染标志在commit时设置
    let mut tx = TransactionProcessor::new();

    // Initially no render pending
    assert!(!tx.take_render_pending());

    // Begin and commit a transaction
    tx.begin(1, 0).unwrap();
    tx.stage_command(StagedCommand {
        opcode: OpCode::SetColor,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 255, 0, 0],
        dirty_fields: DirtyField::Style,
    })
    .unwrap();

    // Before commit, render_pending should be false
    assert!(!tx.render_pending);

    // After commit, render_pending should be true
    let _ = tx.commit(1).unwrap();
    assert!(tx.render_pending);
    assert!(tx.take_render_pending());

    // After taking, should be false again
    assert!(!tx.take_render_pending());
}
