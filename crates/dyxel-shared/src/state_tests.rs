// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for generational ID and dynamic capacity

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn test_node_handle_creation() {
        let handle = NodeHandle::new(5, 1);
        assert_eq!(handle.slot, 5);
        assert_eq!(handle.generation, 1);
        assert!(handle.is_valid());
        
        let invalid = NodeHandle::INVALID;
        assert!(!invalid.is_valid());
    }

    #[test]
    fn test_initial_capacity() {
        let state = SharedState::new();
        assert_eq!(state.get_capacity(), INITIAL_CAPACITY);
    }

    #[test]
    fn test_create_node_with_handle() {
        let mut state = SharedState::new();
        
        // 创建节点
        let handle = state.create_node_with_handle(100);
        assert!(handle.is_some());
        
        let h = handle.unwrap();
        assert_eq!(h.slot, 0); // 第一个节点应该是 slot 0
        assert_eq!(h.generation, 0); // 初始代际为 0
        
        // 验证节点存在
        assert!(state.get_node_by_handle(h).is_some());
    }

    #[test]
    fn test_generational_id_prevents_stale_access() {
        let mut state = SharedState::new();
        
        // 创建节点
        let handle1 = state.create_node_with_handle(100).unwrap();
        let slot = handle1.slot;
        
        // 删除节点
        assert!(state.remove_node_with_handle(handle1));
        
        // 尝试用旧 handle 访问（应该失败）
        assert!(!state.verify_handle(handle1));
        assert!(state.get_node_by_handle(handle1).is_none());
        
        // 创建新节点（应该复用同一 slot，但代际+1）
        let handle2 = state.create_node_with_handle(101).unwrap();
        assert_eq!(handle2.slot, slot);
        assert_eq!(handle2.generation, 1); // 代际增加
        
        // 旧 handle 仍然无效
        assert!(!state.verify_handle(handle1));
        assert!(state.verify_handle(handle2));
    }

    #[test]
    fn test_id_recycling() {
        let mut state = SharedState::new();
        
        // 创建一些节点
        let h1 = state.create_node_with_handle(1).unwrap();
        let h2 = state.create_node_with_handle(2).unwrap();
        let h3 = state.create_node_with_handle(3).unwrap();
        
        // 删除中间的
        state.remove_node_with_handle(h2);
        
        // 创建新节点（应该复用 slot 1）
        let h4 = state.create_node_with_handle(4).unwrap();
        assert_eq!(h4.slot, h2.slot); // 复用了 slot 1
        assert_eq!(h4.generation, 1); // 代际+1
    }

    #[test]
    fn test_capacity_expansion() {
        let mut state = SharedState::new();
        assert_eq!(state.get_capacity(), INITIAL_CAPACITY); // 256
        
        // 扩容到 512
        assert!(state.expand_capacity(512).is_ok());
        assert_eq!(state.get_capacity(), 512);
        
        // 扩容到 1024
        assert!(state.expand_capacity(1024).is_ok());
        assert_eq!(state.get_capacity(), 1024);
        
        // 不能缩容
        assert!(state.expand_capacity(512).is_err());
        
        // 不能超过最大容量
        assert!(state.expand_capacity(5000).is_err());
    }

    #[test]
    fn test_pre_expansion_threshold() {
        let mut state = SharedState::new();
        
        // 创建节点直到超过 80% 阈值
        // 256 * 0.8 = 204.8，所以创建 205 个节点应该触发预扩容
        for i in 0..205 {
            state.create_node_with_handle(i);
        }
        
        // 此时应该建议预扩容
        assert!(state.should_pre_expand());
        
        // 执行扩容
        assert!(state.auto_expand());
        assert_eq!(state.get_capacity(), 512);
    }

    #[test]
    fn test_stats() {
        let mut state = SharedState::new();
        
        // 创建节点
        let h1 = state.create_node_with_handle(1).unwrap();
        state.create_node_with_handle(2).unwrap();
        state.create_node_with_handle(3).unwrap();
        
        // 删除一个
        state.remove_node_with_handle(h1);
        
        let stats = state.get_stats();
        assert_eq!(stats.capacity, INITIAL_CAPACITY);
        assert_eq!(stats.active_count, 2);
        assert_eq!(stats.free_count, 1);
        assert_eq!(stats.total_created, 3);
    }

    #[test]
    fn test_generation_wrapping() {
        let mut state = SharedState::new();
        
        // 创建并反复删除同一 slot
        let mut handle = state.create_node_with_handle(1).unwrap();
        let slot = handle.slot;
        
        for i in 0..10 {
            state.remove_node_with_handle(handle);
            handle = state.create_node_with_handle(i + 2).unwrap();
            assert_eq!(handle.slot, slot);
            assert_eq!(handle.generation, i + 1);
        }
    }
}

// Integration tests for capacity monitoring
#[cfg(test)]
mod integration_tests {
    use crate::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 模拟真实场景：创建-删除-复用循环
    #[test]
    fn test_real_world_usage_pattern() {
        let mut state = SharedState::new();
        
        // Phase 1: 初始创建 100 个节点
        let mut handles = Vec::new();
        for i in 0..100 {
            handles.push(state.create_node_with_handle(i).unwrap());
        }
        assert_eq!(state.get_stats().active_count, 100);
        
        // Phase 2: 删除一半
        for h in handles.drain(50..) {
            state.remove_node_with_handle(h);
        }
        assert_eq!(state.get_stats().active_count, 50);
        assert_eq!(state.get_stats().free_count, 50);
        
        // Phase 3: 创建新节点（应该复用被删除的 slot）
        for i in 100..150 {
            let new_handle = state.create_node_with_handle(i).unwrap();
            // 新节点的 slot 应该 <= 99（复用了 50-99 的 slot）
            assert!(new_handle.slot < 100);
            assert!(new_handle.generation > 0); // 代际增加
            handles.push(new_handle);
        }
        
        assert_eq!(state.get_stats().active_count, 100);
    }

    /// 测试扩容边界条件
    #[test]
    fn test_capacity_boundary() {
        let mut state = SharedState::new();
        
        // 创建到刚好低于 80% 阈值
        let threshold = (INITIAL_CAPACITY as f32 * 0.8) as u32;
        for i in 0..threshold {
            state.create_node_with_handle(i);
        }
        
        // 不应该建议预扩容
        assert!(!state.should_pre_expand());
        
        // 再创建一个，超过阈值
        state.create_node_with_handle(threshold);
        
        // 现在应该建议预扩容
        assert!(state.should_pre_expand());
    }

    /// 测试最大容量限制
    #[test]
    fn test_max_capacity_limit() {
        let mut state = SharedState::new();
        
        // 扩容到最大
        state.expand_capacity(MAX_CAPACITY).unwrap();
        assert_eq!(state.get_capacity(), MAX_CAPACITY);
        
        // 创建到最大容量
        let mut handles = Vec::new();
        for i in 0..MAX_CAPACITY as u32 {
            if let Some(h) = state.create_node_with_handle(i) {
                handles.push(h);
            } else {
                // LRU 回收应该发生
                break;
            }
        }
        
        // 验证活跃节点数量
        let stats = state.get_stats();
        assert!(stats.active_count <= MAX_CAPACITY);
    }

    /// 测试并发创建（模拟高负载）
    #[test]
    fn test_rapid_create_delete() {
        let mut state = SharedState::new();
        
        // 快速创建和删除 1000 次
        for i in 0..1000 {
            let h = state.create_node_with_handle(i).unwrap();
            state.remove_node_with_handle(h);
        }
        
        // 最终应该没有活跃节点
        assert_eq!(state.get_stats().active_count, 0);
        
        // 但应该有大量回收的 ID
        assert!(state.get_stats().free_count > 0);
        
        // 验证代际计数
        let max_generation = state.get_generations()
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        assert!(max_generation > 0);
    }
}
