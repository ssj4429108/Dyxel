// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for LayoutRegistry system
//! 
//! 验证标准:
//! 1. WASM 可以获取任意节点布局（延迟 1 帧）
//! 2. 脏标记机制正常工作（避免重复读取未变更节点）

use dyxel_core::transaction::*;
use dyxel_shared::{OpCode, DirtyField};

/// 模拟布局同步过程
/// Host 侧布局计算完成后，sync_layout_to_wasm 将结果写入共享内存
#[test]
fn test_layout_registry_basic() {
    // 1. Create nodes via transaction
    let mut tx = TransactionProcessor::new();
    tx.begin(1, 0).unwrap();
    
    // Create a node
    tx.stage_command(StagedCommand {
        opcode: OpCode::CreateNode,
        node_id: 0,
        payload: vec![0, 0, 0, 0],
        dirty_fields: DirtyField::None,
    }).unwrap();
    
    // Set size (affects layout)
    tx.stage_command(StagedCommand {
        opcode: OpCode::SetWidth,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 1, 0x64, 0, 0, 0], // 100px
        dirty_fields: DirtyField::Size,
    }).unwrap();
    
    tx.stage_command(StagedCommand {
        opcode: OpCode::SetHeight,
        node_id: 0,
        payload: vec![0, 0, 0, 0, 1, 0x64, 0, 0, 0], // 100px
        dirty_fields: DirtyField::Size,
    }).unwrap();
    
    let cmds = tx.commit(1).unwrap();
    assert_eq!(cmds.len(), 3); // Create + Width + Height
    
    // Verify dirty tracker marks layout as dirty
    assert!(tx.dirty_tracker.is_node_dirty(0));
    let fields = tx.dirty_tracker.get_dirty_fields(0);
    assert!(fields & DirtyField::Size.bits() != 0);
}

#[test]
fn test_layout_registry_opcodes_handled() {
    //! 验证 LayoutRegistry 操作码被正确处理（不触发 transaction staging）
    let mut tx = TransactionProcessor::new();
    
    // These opcodes should be handled directly, not staged
    let layout_ops = [
        OpCode::GetLayout,
        OpCode::IsLayoutDirty,
        OpCode::ClearLayoutDirty,
        OpCode::GetLayoutBatch,
    ];
    
    for op in layout_ops {
        // These ops don't produce dirty fields
        let dirty = get_dirty_field_for_opcode(&op);
        assert_eq!(dirty, DirtyField::None, "{:?} should not produce dirty fields", op);
    }
}

#[test]
fn test_layout_dirty_tracking_per_node() {
    //! 验证每个节点有独立的脏标记
    let mut tracker = DirtyTracker::new();
    
    // Mark node 0 as dirty (Style)
    tracker.mark_dirty(0, DirtyField::Style);
    
    // Mark node 5 as dirty (Size)
    tracker.mark_dirty(5, DirtyField::Size);
    
    // Both should be dirty
    assert!(tracker.is_node_dirty(0));
    assert!(tracker.is_node_dirty(5));
    assert!(!tracker.is_node_dirty(1));
    
    // Clear node 0
    tracker.node_dirty_fields.remove(&0);
    let word_idx = 0;
    tracker.node_bitset[word_idx] &= !(1 << 0);
    
    // Node 0 should be clean, node 5 still dirty
    // Note: is_node_dirty checks bitset, we manually cleared it
    assert!(!tracker.is_node_dirty(0));
    assert!(tracker.is_node_dirty(5));
}

#[test]
fn test_layout_read_after_write_pattern() {
    //! 验证布局读取的典型使用模式
    // Pattern:
    // 1. Host: 计算布局
    // 2. Host: sync_layout_to_wasm 写入共享内存
    // 3. Host: 设置 dirty_mask
    // 4. WASM: tick 中检查 is_layout_dirty
    // 5. WASM: get_layout 读取位置/大小
    // 6. WASM: clear_layout_dirty 清除标记
    
    let mut tracker = DirtyTracker::new();
    
    // Step 1-2-3: Host layout sync simulation
    tracker.mark_dirty(10, DirtyField::Position);
    
    // Step 4: WASM checks dirty
    assert!(tracker.is_node_dirty(10), "Node 10 should be dirty after sync");
    
    // Step 5: WASM reads layout (simulated)
    let fields = tracker.get_dirty_fields(10);
    assert!(fields & DirtyField::Position.bits() != 0);
    
    // Step 6: WASM clears dirty
    tracker.node_dirty_fields.remove(&10);
    tracker.node_bitset[0] &= !(1 << 10);
    
    assert!(!tracker.is_node_dirty(10), "Node 10 should be clean after clear");
}

#[test]
fn test_batch_layout_operations() {
    //! 验证批量布局操作
    let mut tracker = DirtyTracker::new();
    
    // Mark multiple nodes as dirty
    for i in 0..100 {
        tracker.mark_dirty(i, DirtyField::Position);
    }
    
    // Verify all are dirty
    for i in 0..100 {
        assert!(tracker.is_node_dirty(i), "Node {} should be dirty", i);
    }
    
    // Clear batch
    tracker.clear();
    
    // Verify all clean
    for i in 0..100 {
        assert!(!tracker.is_node_dirty(i), "Node {} should be clean after clear", i);
    }
}
