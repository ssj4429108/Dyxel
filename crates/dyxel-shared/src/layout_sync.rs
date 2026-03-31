// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Layout synchronization helper
//! 
//! This module provides a mechanism for Render thread to notify Core thread
//! that layout has been computed and nodes need to be marked as dirty.

use std::sync::Mutex;

/// Global registry of nodes that need to be marked as dirty after layout computation
static LAYOUT_DIRTY_REGISTRY: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Register nodes that need to be marked as dirty after layout computation
/// Called by Render thread after compute_layout
pub fn register_layout_dirty_nodes(node_ids: &[u32]) {
    if let Ok(mut registry) = LAYOUT_DIRTY_REGISTRY.lock() {
        registry.clear();
        registry.extend_from_slice(node_ids);
        log::debug!("[LayoutSync] Registered {} nodes as layout-dirty", node_ids.len());
    }
}

/// Take the registered nodes and clear the registry
/// Called by Core thread before sync_layout_to_wasm
pub fn take_layout_dirty_nodes() -> Vec<u32> {
    if let Ok(mut registry) = LAYOUT_DIRTY_REGISTRY.lock() {
        let nodes = std::mem::take(&mut *registry);
        if !nodes.is_empty() {
            log::debug!("[LayoutSync] Taking {} layout-dirty nodes", nodes.len());
        }
        nodes
    } else {
        Vec::new()
    }
}

/// Check if there are any pending layout-dirty nodes
pub fn has_layout_dirty_nodes() -> bool {
    if let Ok(registry) = LAYOUT_DIRTY_REGISTRY.lock() {
        !registry.is_empty()
    } else {
        false
    }
}
