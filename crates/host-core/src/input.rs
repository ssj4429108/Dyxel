use std::collections::HashMap;
use kurbo::{Rect as KRect, Vec2, Point};
use taffy::prelude::*;
use crate::state::ViewNode;

pub fn hit_test_recursive(id: u32, point: Vec2, nodes: &HashMap<u32, ViewNode>, taffy: &TaffyTree<()>, parent_pos: Vec2, listeners: &[u32]) -> Option<u32> {
    if let Some(node) = nodes.get(&id) {
        let layout = taffy.layout(node.taffy_node).unwrap();
        let global_pos = parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);
        let rect = KRect::from_origin_size((global_pos.x, global_pos.y), (layout.size.width as f64, layout.size.height as f64));
        for &child_id in node.children.iter().rev() { 
            if let Some(hit) = hit_test_recursive(child_id, point, nodes, taffy, global_pos, listeners) { 
                return Some(hit); 
            } 
        }
        if rect.contains(Point::new(point.x, point.y)) && listeners.contains(&id) { 
            return Some(id); 
        }
    }
    None
}
