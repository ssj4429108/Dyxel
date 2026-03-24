// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

pub use dyxel_shared::{FlexDirection, JustifyContent, AlignItems, PositionType, Role, ViewType};

use std::collections::HashMap;
use taffy::prelude::*;
use peniko::Color;

pub struct ViewNode { 
    pub taffy_node: NodeId, 
    pub color: Color, 
    pub children: Vec<u32>, 
    pub z_index: i32, 
    pub label: String, 
    pub text: String, 
    pub font_size: f32, 
    pub border_radius: f32, 
    pub role: Role, 
    pub view_type: ViewType, 
    pub has_click: bool, 
    pub padding: (f32, f32, f32, f32) 
}

pub struct SharedState { 
    pub taffy: TaffyTree<()>, 
    pub nodes: HashMap<u32, ViewNode>, 
    pub root_id: Option<u32>, 
    pub click_listeners: Vec<u32>, 
    pub font_data: Option<Vec<u8>> 
}

unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

impl SharedState {
    pub fn new() -> Self { 
        Self { 
            taffy: TaffyTree::new(), 
            nodes: HashMap::new(), 
            root_id: None, 
            click_listeners: vec![], 
            font_data: None 
        } 
    }
    
    pub fn create_node(&mut self, id: u32) {
        let taffy_node = self.taffy.new_leaf(Style::default()).unwrap();
        self.nodes.insert(id, ViewNode { 
            taffy_node, 
            color: Color::WHITE, 
            children: vec![], 
            z_index: 0, 
            label: String::new(), 
            text: String::new(), 
            font_size: 16.0, 
            border_radius: 0.0, 
            role: Role::None, 
            view_type: ViewType::Container, 
            has_click: false, 
            padding: (0.0, 0.0, 0.0, 0.0) 
        });
        if self.root_id.is_none() { self.root_id = Some(id); }
    }
    
    pub fn set_view_type(&mut self, id: u32, vt: u32) { 
        if let Some(node) = self.nodes.get_mut(&id) { 
            node.view_type = match vt { 1 => ViewType::Text, 2 => ViewType::Button, _ => ViewType::Container }; 
        } 
    }
    
    pub fn set_text(&mut self, id: u32, text: String) { 
        if let Some(node) = self.nodes.get_mut(&id) { node.text = text; } 
    }
    
    pub fn set_font_size(&mut self, id: u32, size: f32) { 
        if let Some(node) = self.nodes.get_mut(&id) { node.font_size = size; } 
    }
    
    pub fn set_color(&mut self, id: u32, r: u8, g: u8, b: u8) { 
        if let Some(node) = self.nodes.get_mut(&id) { node.color = Color::from_rgb8(r, g, b); } 
    }
    
    pub fn set_width(&mut self, id: u32, dt: u32, v: f32) { 
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.size.width = match dt { 1 => taffy::style::Dimension::length(v), 2 => taffy::style::Dimension::percent(v / 100.0), _ => taffy::style::Dimension::auto() }; 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn set_height(&mut self, id: u32, dt: u32, v: f32) { 
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.size.height = match dt { 1 => taffy::style::Dimension::length(v), 2 => taffy::style::Dimension::percent(v / 100.0), _ => taffy::style::Dimension::auto() }; 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn set_inset(&mut self, id: u32, t: f32, r: f32, b: f32, l: f32) { 
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.inset.top = LengthPercentage::percent(t / 100.0).into(); 
            s.inset.right = LengthPercentage::percent(r / 100.0).into(); 
            s.inset.bottom = LengthPercentage::percent(b / 100.0).into(); 
            s.inset.left = LengthPercentage::percent(l / 100.0).into(); 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn set_position(&mut self, id: u32, p: u32) { 
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.position = if p == 1 { Position::Absolute } else { Position::Relative }; 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn set_flex_direction(&mut self, id: u32, dir: u32) { 
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.flex_direction = match dir { 
                1 => taffy::prelude::FlexDirection::Column, 
                2 => taffy::prelude::FlexDirection::RowReverse, 
                3 => taffy::prelude::FlexDirection::ColumnReverse, 
                _ => taffy::prelude::FlexDirection::Row 
            }; 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn set_justify_content(&mut self, id: u32, j: u32) { 
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.justify_content = Some(match j { 
                1 => taffy::prelude::JustifyContent::Center, 
                2 => taffy::prelude::JustifyContent::FlexEnd, 
                3 => taffy::prelude::JustifyContent::SpaceBetween, 
                4 => taffy::prelude::JustifyContent::SpaceAround, 
                5 => taffy::prelude::JustifyContent::SpaceEvenly, 
                _ => taffy::prelude::JustifyContent::FlexStart 
            }); 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn set_align_items(&mut self, id: u32, a: u32) { 
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.align_items = Some(match a { 
                1 => taffy::prelude::AlignItems::Center, 
                2 => taffy::prelude::AlignItems::FlexEnd, 
                3 => taffy::prelude::AlignItems::Stretch, 
                _ => taffy::prelude::AlignItems::FlexStart 
            }); 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn set_flex_grow(&mut self, id: u32, grow: f32) { 
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.flex_grow = grow; 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn set_z_index(&mut self, id: u32, z: i32) { 
        if let Some(node) = self.nodes.get_mut(&id) { node.z_index = z; } 
    }
    
    pub fn set_border_radius(&mut self, id: u32, r: f32) { 
        if let Some(node) = self.nodes.get_mut(&id) { node.border_radius = r; } 
    }
    
    pub fn set_padding(&mut self, id: u32, t: f32, r: f32, b: f32, l: f32) { 
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.padding.top = LengthPercentage::length(t).into(); 
            s.padding.right = LengthPercentage::length(r).into(); 
            s.padding.bottom = LengthPercentage::length(b).into(); 
            s.padding.left = LengthPercentage::length(l).into(); 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn attach_click(&mut self, id: u32) { 
        self.click_listeners.push(id); 
        if let Some(node) = self.nodes.get_mut(&id) { node.has_click = true; } 
    }
    
    pub fn add_child(&mut self, pid: u32, cid: u32) { 
        let c_tn = self.nodes.get(&cid).map(|n| n.taffy_node); 
        let p_tn = self.nodes.get(&pid).map(|n| n.taffy_node); 
        if let (Some(ptn), Some(ctn)) = (p_tn, c_tn) { 
            if let Some(parent) = self.nodes.get_mut(&pid) { parent.children.push(cid); } 
            self.taffy.add_child(ptn, ctn).unwrap(); 
        } 
    }
    
    pub fn set_font_data(&mut self, data: Vec<u8>) { self.font_data = Some(data); }
}
