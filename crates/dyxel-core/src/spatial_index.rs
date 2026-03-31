// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Spatial Index for O(log N) Hit Testing
//!
//! Uses a simple grid-based spatial hash for dynamic scenes.
//! For static UI, a BVH (Bounding Volume Hierarchy) would be better.

use std::collections::HashMap;

const GRID_CELL_SIZE: f32 = 100.0; // 100x100 logical pixels per cell

/// Spatial index for fast hit testing
pub struct SpatialIndex {
    /// Grid cells: cell_key -> [node_ids]
    grid: HashMap<(i32, i32), Vec<u32>>,
    /// Node bounds: node_id -> (x, y, width, height)
    bounds: HashMap<u32, (f32, f32, f32, f32)>,
}

impl SpatialIndex {
    pub fn new() -> Self {
        Self {
            grid: HashMap::new(),
            bounds: HashMap::new(),
        }
    }

    /// Insert or update a node
    pub fn insert(&mut self, node_id: u32, x: f32, y: f32, width: f32, height: f32) {
        // Remove old position
        self.remove(node_id);
        
        // Store bounds
        self.bounds.insert(node_id, (x, y, width, height));
        
        // Insert into grid cells
        let min_x = ((x) / GRID_CELL_SIZE).floor() as i32;
        let max_x = ((x + width) / GRID_CELL_SIZE).ceil() as i32;
        let min_y = ((y) / GRID_CELL_SIZE).floor() as i32;
        let max_y = ((y + height) / GRID_CELL_SIZE).ceil() as i32;
        
        for cx in min_x..=max_x {
            for cy in min_y..=max_y {
                self.grid.entry((cx, cy)).or_default().push(node_id);
            }
        }
    }

    /// Remove a node
    pub fn remove(&mut self, node_id: u32) {
        if let Some((x, y, w, h)) = self.bounds.remove(&node_id) {
            let min_x = (x / GRID_CELL_SIZE).floor() as i32;
            let max_x = ((x + w) / GRID_CELL_SIZE).ceil() as i32;
            let min_y = (y / GRID_CELL_SIZE).floor() as i32;
            let max_y = ((y + h) / GRID_CELL_SIZE).ceil() as i32;
            
            for cx in min_x..=max_x {
                for cy in min_y..=max_y {
                    if let Some(cell) = self.grid.get_mut(&(cx, cy)) {
                        cell.retain(|&id| id != node_id);
                    }
                }
            }
        }
    }

    /// Hit test: return nodes at point (sorted by z-order, highest first)
    pub fn hit_test(&self, x: f32, y: f32) -> Vec<u32> {
        let cx = (x / GRID_CELL_SIZE).floor() as i32;
        let cy = (y / GRID_CELL_SIZE).floor() as i32;
        
        let mut result = Vec::new();
        
        // Check cell and neighbors (for nodes crossing cell boundaries)
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(cell) = self.grid.get(&(cx + dx, cy + dy)) {
                    for &node_id in cell {
                        if let Some((nx, ny, nw, nh)) = self.bounds.get(&node_id) {
                            if x >= *nx && x <= nx + nw && y >= *ny && y <= ny + nh {
                                result.push(node_id);
                            }
                        }
                    }
                }
            }
        }
        
        // Sort by node_id descending (higher ID = higher z-order)
        result.sort_by(|a, b| b.cmp(a));
        result
    }
}

/// Hierarchical hit test result with bubbling path
pub struct HitTestResult {
    /// The direct hit node
    pub target: u32,
    /// Path from target to root for event bubbling [target, parent, grandparent, ..., root]
    pub bubble_path: Vec<u32>,
}

/// Scene tree with spatial indexing
pub struct SceneTree {
    spatial: SpatialIndex,
    /// Parent relationship: child -> parent
    parents: HashMap<u32, u32>,
    /// Root node id
    root: Option<u32>,
}

impl SceneTree {
    pub fn new() -> Self {
        Self {
            spatial: SpatialIndex::new(),
            parents: HashMap::new(),
            root: None,
        }
    }

    /// Update node layout
    pub fn update_node(&mut self, id: u32, x: f32, y: f32, width: f32, height: f32) {
        self.spatial.insert(id, x, y, width, height);
    }

    /// Set parent relationship
    pub fn set_parent(&mut self, child: u32, parent: u32) {
        self.parents.insert(child, parent);
    }

    /// Hit test with bubble path
    pub fn hit_test(&self, x: f32, y: f32) -> Option<HitTestResult> {
        let hits = self.spatial.hit_test(x, y);
        
        // First hit is the top-most node (highest z-order)
        hits.first().map(|&target| {
            let mut bubble_path = vec![target];
            let mut current = target;
            
            // Build path to root
            while let Some(&parent) = self.parents.get(&current) {
                bubble_path.push(parent);
                current = parent;
            }
            
            HitTestResult { target, bubble_path }
        })
    }
}
