// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Spatial Index-based Hit Tester for O(1) Hit Testing
//!
//! Replaces O(N) linear search with grid-based spatial hashing.
//! Suitable for scenes with 1000+ nodes.

use std::collections::HashMap;
use crate::{HitTester, HitTestResult};

/// Grid cell size in logical pixels
const GRID_CELL_SIZE: f32 = 100.0;

/// Spatial index for fast hit testing
pub struct SpatialHitTester {
    /// Grid cells: (cell_x, cell_y) -> [node_ids]
    grid: HashMap<(i32, i32), Vec<u32>>,
    /// Node bounds and metadata
    nodes: HashMap<u32, NodeData>,
    /// Shared buffer pointer for layout updates
    shared_buffer_ptr: *const dyxel_shared::SharedBuffer,
    /// Last synced max_node_id
    last_synced_max_id: u32,
}

/// Node data stored in spatial index
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct NodeData {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    /// Which grid cells this node occupies
    min_cell_x: i32,
    max_cell_x: i32,
    min_cell_y: i32,
    max_cell_y: i32,
}

// SAFETY: The pointer is valid as long as the SharedBuffer exists
unsafe impl Send for SpatialHitTester {}

impl SpatialHitTester {
    /// Create a new spatial hit tester
    /// 
    /// # Safety
    /// The pointer must be valid for the lifetime of this hit tester
    pub unsafe fn new(shared_buffer_ptr: *const dyxel_shared::SharedBuffer) -> Self {
        Self {
            grid: HashMap::new(),
            nodes: HashMap::new(),
            shared_buffer_ptr,
            last_synced_max_id: 0,
        }
    }

    /// Incrementally sync with shared buffer
    /// 
    /// Call this before hit testing to pick up new nodes
    pub fn sync(&mut self) {
        self.do_sync();
    }

    /// Internal sync implementation
    fn do_sync(&mut self) {
        unsafe {
            // Use unaligned read to avoid alignment issues with #[repr(C, align(16))]
            let max_node_id_ptr = std::ptr::addr_of!((*self.shared_buffer_ptr).max_node_id);
            let current_max = std::ptr::read_unaligned(max_node_id_ptr);
            
            // Sync spatial index with shared buffer
            
            // Process new nodes
            for id in (self.last_synced_max_id + 1)..=current_max {
                self.add_node(id);
            }
            
            // Check existing nodes for layout changes
            // Use dirty_mask from shared buffer to detect changed nodes efficiently
            self.update_changed_nodes();
            
            self.last_synced_max_id = current_max;
            
            log::debug!("SpatialHitTester: sync complete, total nodes in index: {}", self.nodes.len());
        }
    }
    
    /// Update nodes that have changed their layout
    /// Uses dirty_mask from shared buffer for efficient change detection
    unsafe fn update_changed_nodes(&mut self) {
        let dirty_mask = &(*self.shared_buffer_ptr).dirty_mask;
        
        // Collect node IDs that need updating
        let mut nodes_to_update: Vec<u32> = Vec::new();
        let _checked_count = 0;
        let mut dirty_mask_hits = 0;
        let mut bounds_changed_hits = 0;
        
        for (node_id, data) in &self.nodes {
            let _ = _checked_count;
            let word_idx = (node_id / 32) as usize;
            let bit_idx = node_id % 32;
            
            // Check if this node is marked as dirty in the shared buffer
            if word_idx < dirty_mask.len() && (dirty_mask[word_idx] >> bit_idx) & 1 != 0 {
                nodes_to_update.push(*node_id);
                dirty_mask_hits += 1;
                continue;
            }
            
            // Also check if layout has changed (fallback for cases where dirty_mask isn't set)
            if *node_id as usize >= dyxel_shared::MAX_NODES {
                continue;
            }
            let layout = (*self.shared_buffer_ptr).layout_results[*node_id as usize];
            
            // Compare current layout with stored data
            let bounds_changed = 
                (layout.x - data.x).abs() > 0.01 ||
                (layout.y - data.y).abs() > 0.01 ||
                (layout.width - data.width).abs() > 0.01 ||
                (layout.height - data.height).abs() > 0.01;
            
            if bounds_changed {
                nodes_to_update.push(*node_id);
                bounds_changed_hits += 1;
                log::debug!("[SpatialHitTester] Node {} bounds changed: stored ({:.1},{:.1},{:.1},{:.1}) vs current ({:.1},{:.1},{:.1},{:.1})",
                    node_id, data.x, data.y, data.width, data.height, layout.x, layout.y, layout.width, layout.height);
            }
        }
        
        // Update nodes that changed
        if !nodes_to_update.is_empty() {
            log::debug!("[SpatialHitTester] Updating {} nodes (dirty_mask: {}, bounds_changed: {})",
                nodes_to_update.len(), dirty_mask_hits, bounds_changed_hits);
            for node_id in nodes_to_update {
                self.remove_node(node_id);
                self.add_node(node_id);
            }
        }
    }

    /// Full rebuild of spatial index
    /// 
    /// Use this when layout changes significantly (e.g., window resize)
    pub fn rebuild(&mut self) {
        self.grid.clear();
        self.nodes.clear();
        self.last_synced_max_id = 0;
        self.do_sync();
    }

    /// Add a single node to spatial index
    unsafe fn add_node(&mut self, node_id: u32) {
        // Bounds check to prevent out-of-bounds access
        if node_id as usize >= dyxel_shared::MAX_NODES {
            return;
        }
        let layout = (*self.shared_buffer_ptr).layout_results[node_id as usize];
        
        // Skip zero-size nodes
        if layout.width <= 0.0 || layout.height <= 0.0 {
            // Skip zero-size nodes
            return;
        }
        
        // Node added to spatial index

        // Calculate grid cells this node occupies
        let min_cell_x = (layout.x / GRID_CELL_SIZE).floor() as i32;
        let max_cell_x = ((layout.x + layout.width) / GRID_CELL_SIZE).ceil() as i32;
        let min_cell_y = (layout.y / GRID_CELL_SIZE).floor() as i32;
        let max_cell_y = ((layout.y + layout.height) / GRID_CELL_SIZE).ceil() as i32;

        let data = NodeData {
            x: layout.x,
            y: layout.y,
            width: layout.width,
            height: layout.height,
            min_cell_x,
            max_cell_x,
            min_cell_y,
            max_cell_y,
        };

        // Insert into grid cells
        for cx in min_cell_x..=max_cell_x {
            for cy in min_cell_y..=max_cell_y {
                self.grid.entry((cx, cy)).or_default().push(node_id);
            }
        }

        self.nodes.insert(node_id, data);
    }

    /// Remove a node from spatial index
    fn remove_node(&mut self, node_id: u32) {
        if let Some(data) = self.nodes.remove(&node_id) {
            for cx in data.min_cell_x..=data.max_cell_x {
                for cy in data.min_cell_y..=data.max_cell_y {
                    if let Some(cell) = self.grid.get_mut(&(cx, cy)) {
                        cell.retain(|&id| id != node_id);
                    }
                }
            }
        }
    }

    /// Get approximate memory usage in bytes
    pub fn memory_usage(&self) -> usize {
        let grid_size = self.grid.capacity() * (std::mem::size_of::<(i32, i32)>() + std::mem::size_of::<Vec<u32>>());
        let cell_size: usize = self.grid.values().map(|v| v.capacity() * std::mem::size_of::<u32>()).sum();
        let node_size = self.nodes.capacity() * (std::mem::size_of::<u32>() + std::mem::size_of::<NodeData>());
        grid_size + cell_size + node_size
    }

    /// Get stats for debugging
    pub fn stats(&self) -> SpatialStats {
        SpatialStats {
            num_nodes: self.nodes.len(),
            num_cells: self.grid.len(),
            avg_nodes_per_cell: if self.grid.is_empty() {
                0.0
            } else {
                self.grid.values().map(|v| v.len()).sum::<usize>() as f32 / self.grid.len() as f32
            },
        }
    }
}

/// Statistics for spatial index
#[derive(Debug, Clone, Copy)]
pub struct SpatialStats {
    pub num_nodes: usize,
    pub num_cells: usize,
    pub avg_nodes_per_cell: f32,
}

impl HitTester for SpatialHitTester {
    fn sync(&mut self) {
        // Call the inherent method, not trait method
        SpatialHitTester::sync(self);
    }
    
    fn hit_test(&self, x: f32, y: f32) -> HitTestResult {
        // Calculate grid cell for point
        let cell_x = (x / GRID_CELL_SIZE).floor() as i32;
        let cell_y = (y / GRID_CELL_SIZE).floor() as i32;
        
        let mut best_node: Option<(u32, NodeData)> = None;
        let mut checked_nodes = 0;
        let mut in_bounds_nodes = 0;

        // Check cell and neighbors (for nodes crossing cell boundaries)
        for dx in -1..=1 {
            for dy in -1..=1 {
                let check_cell = (cell_x + dx, cell_y + dy);
                if let Some(cell) = self.grid.get(&check_cell) {
                    for &node_id in cell {
                        checked_nodes += 1;
                        if let Some(&data) = self.nodes.get(&node_id) {
                            // Check if point is inside node bounds
                            if x >= data.x
                                && x <= data.x + data.width
                                && y >= data.y
                                && y <= data.y + data.height
                            {
                                in_bounds_nodes += 1;
                                // Keep highest node ID (z-order)
                                if best_node.map(|(id, _)| node_id > id).unwrap_or(true) {
                                    best_node = Some((node_id, data));
                                }
                            }
                        }
                    }
                }
            }
        }

        log::trace!("[SpatialHitTester] Hit test at ({:.1},{:.1}): cell ({},{}), checked {} nodes, {} in bounds, best: {:?}",
            x, y, cell_x, cell_y, checked_nodes, in_bounds_nodes, best_node.map(|(id, _)| id));

        match best_node {
            Some((id, data)) => HitTestResult::hit(id, x - data.x, y - data.y),
            None => HitTestResult::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spatial_hit_tester_basic() {
        // Create a mock shared buffer
        use std::alloc::{alloc, Layout};
        
        unsafe {
            let layout = Layout::new::<dyxel_shared::SharedBuffer>();
            let ptr = alloc(layout) as *mut dyxel_shared::SharedBuffer;
            
            // Initialize with some layout data
            (*ptr).max_node_id = 3;
            (*ptr).layout_results[1] = dyxel_shared::LayoutResult {
                x: 0.0, y: 0.0, width: 100.0, height: 100.0,
            };
            (*ptr).layout_results[2] = dyxel_shared::LayoutResult {
                x: 50.0, y: 50.0, width: 100.0, height: 100.0,
            };
            (*ptr).layout_results[3] = dyxel_shared::LayoutResult {
                x: 200.0, y: 200.0, width: 50.0, height: 50.0,
            };

            let mut tester = SpatialHitTester::new(ptr);
            tester.sync();

            // Hit in first rect
            let result = tester.hit_test(25.0, 25.0);
            assert_eq!(result.node_id, 1);

            // Hit in overlapping area (higher ID wins)
            let result = tester.hit_test(75.0, 75.0);
            assert_eq!(result.node_id, 2);

            // No hit
            let result = tester.hit_test(500.0, 500.0);
            assert_eq!(result.node_id, 0);

            // Hit third rect
            let result = tester.hit_test(220.0, 220.0);
            assert_eq!(result.node_id, 3);
        }
    }
}
