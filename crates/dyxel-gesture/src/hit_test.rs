// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hit Testing
//! 
//! Determines which UI node is under a pointer position.

/// Result of a hit test
#[derive(Debug, Clone, Copy)]
pub struct HitTestResult {
    /// The node ID that was hit (0 if no hit)
    pub node_id: u32,
    /// X position in local coordinates
    pub local_x: f32,
    /// Y position in local coordinates
    pub local_y: f32,
    /// Whether the hit is inside the node's bounds
    pub is_inside: bool,
}

impl HitTestResult {
    /// Create a hit result with no hit
    pub fn none() -> Self {
        Self {
            node_id: 0,
            local_x: 0.0,
            local_y: 0.0,
            is_inside: false,
        }
    }

    /// Create a hit result with a hit
    pub fn hit(node_id: u32, local_x: f32, local_y: f32) -> Self {
        Self {
            node_id,
            local_x,
            local_y,
            is_inside: true,
        }
    }
}

/// Trait for hit testing implementations
/// 
/// The Host layer provides an implementation that uses the layout tree
pub trait HitTester: Send {
    /// Hit test at the given position
    /// 
    /// Returns the top-most node under the position (in z-order)
    fn hit_test(&self, x: f32, y: f32) -> HitTestResult;
    
    /// Sync with data source (for incremental spatial index)
    /// 
    /// Default implementation does nothing. Spatial index implementations
    /// should update their internal structures here.
    fn sync(&mut self) {}
}

/// No-op hit tester for testing
pub struct NoOpHitTester;

impl HitTester for NoOpHitTester {
    fn hit_test(&self, _x: f32, _y: f32) -> HitTestResult {
        HitTestResult::none()
    }
}

/// Simple hit tester based on a list of rectangles
/// 
/// Used for testing and simple use cases
pub struct RectHitTester {
    /// List of (node_id, x, y, width, height)
    rects: Vec<(u32, f32, f32, f32, f32)>,
}

impl RectHitTester {
    pub fn new() -> Self {
        Self { rects: Vec::new() }
    }

    pub fn add_rect(&mut self, node_id: u32, x: f32, y: f32, width: f32, height: f32) {
        self.rects.push((node_id, x, y, width, height));
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }
}

impl HitTester for RectHitTester {
    fn hit_test(&self, x: f32, y: f32) -> HitTestResult {
        // Iterate in reverse to find top-most (highest z-order)
        for &(node_id, rx, ry, rw, rh) in self.rects.iter().rev() {
            if x >= rx && x <= rx + rw && y >= ry && y <= ry + rh {
                return HitTestResult::hit(node_id, x - rx, y - ry);
            }
        }
        HitTestResult::none()
    }
}

/// Layout-based hit tester using SharedBuffer
/// 
/// This is the production implementation that uses the layout results
/// from the SharedBuffer.
pub struct LayoutHitTester {
    /// Pointer to shared buffer
    shared_buffer_ptr: *const dyxel_shared::SharedBuffer,
}

// SAFETY: The pointer is valid as long as the SharedBuffer exists
// The Host layer ensures the SharedBuffer outlives the GestureRouter
unsafe impl Send for LayoutHitTester {}

impl LayoutHitTester {
    /// Create a new layout hit tester
    /// 
    /// # Safety
    /// The pointer must be valid for the lifetime of this hit tester
    pub unsafe fn new(shared_buffer_ptr: *const dyxel_shared::SharedBuffer) -> Self {
        Self { shared_buffer_ptr }
    }

    /// Get max node ID from shared buffer
    unsafe fn get_max_node_id(&self) -> u32 {
        (*self.shared_buffer_ptr).max_node_id
    }

    /// Get layout for a node
    unsafe fn get_layout(&self, node_id: u32) -> dyxel_shared::LayoutResult {
        (*self.shared_buffer_ptr).layout_results[node_id as usize]
    }
}

impl HitTester for LayoutHitTester {
    fn hit_test(&self, x: f32, y: f32) -> HitTestResult {
        unsafe {
            let max_id = self.get_max_node_id();
            
            // Check nodes from highest ID to lowest (z-order)
            for id in (1..=max_id).rev() {
                let layout = self.get_layout(id);
                
                // Skip nodes with zero size
                if layout.width <= 0.0 || layout.height <= 0.0 {
                    continue;
                }

                // Check if point is inside bounds
                if x >= layout.x 
                    && x <= layout.x + layout.width
                    && y >= layout.y 
                    && y <= layout.y + layout.height 
                {
                    return HitTestResult::hit(
                        id,
                        x - layout.x,
                        y - layout.y,
                    );
                }
            }
            
            HitTestResult::none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hit_test_result() {
        let none = HitTestResult::none();
        assert_eq!(none.node_id, 0);
        assert!(!none.is_inside);

        let hit = HitTestResult::hit(5, 10.0, 20.0);
        assert_eq!(hit.node_id, 5);
        assert!(hit.is_inside);
        assert_eq!(hit.local_x, 10.0);
        assert_eq!(hit.local_y, 20.0);
    }

    #[test]
    fn test_rect_hit_tester() {
        let mut tester = RectHitTester::new();
        tester.add_rect(1, 0.0, 0.0, 100.0, 100.0);
        tester.add_rect(2, 50.0, 50.0, 100.0, 100.0);

        // Hit in first rect only
        let result = tester.hit_test(25.0, 25.0);
        assert_eq!(result.node_id, 1);

        // Hit in second rect (higher z-order, checked first)
        let result = tester.hit_test(75.0, 75.0);
        assert_eq!(result.node_id, 2);

        // No hit
        let result = tester.hit_test(200.0, 200.0);
        assert_eq!(result.node_id, 0);
    }

    #[test]
    fn test_no_op_hit_tester() {
        let tester = NoOpHitTester;
        let result = tester.hit_test(100.0, 100.0);
        assert_eq!(result.node_id, 0);
    }
}
