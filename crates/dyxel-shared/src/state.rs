// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::HashMap;
use taffy::prelude::*;
use peniko::Color;
use crate::types::{ViewType, Role};

pub struct ViewNode { 
    pub taffy_node: NodeId, 
    pub color: Color, 
    pub children: Vec<u32>, 
    pub z_index: i32, 
    pub label: String, 
    pub text: String,
    pub font_size: f32,
    pub font_family: String,
    pub font_weight: u16,
    pub border_radius: f32, 
    pub role: Role, 
    pub view_type: ViewType, 
    pub has_click: bool, 
    pub padding: (f32, f32, f32, f32),
    /// Dirty field tracking for command deduplication
    pub dirty_fields: u8,
}

pub struct SharedState { 
    pub taffy: TaffyTree<()>, 
    pub nodes: HashMap<u32, ViewNode>, 
    pub root_id: Option<u32>, 
    pub click_listeners: Vec<u32>, 
    pub font_data: Option<Vec<u8>>,
    // Track WASM session for hot restart detection
    wasm_base_id: Option<u32>,
    last_seen_id: Option<u32>,
    // ID mapping for hot restart: maps WASM ID -> Host ID
    id_map: HashMap<u32, u32>,
    next_host_id: u32,
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
            font_data: None,
            wasm_base_id: None,
            last_seen_id: None,
            id_map: HashMap::new(),
            next_host_id: 0,
        } 
    }

    /// Clear all state - used when WASM is reloaded (hot restart)
    pub fn clear(&mut self) {
        let node_count = self.nodes.len();
        if node_count > 0 {
            log::info!("SharedState: clearing {} nodes", node_count);
            self.nodes.clear();
            self.taffy = TaffyTree::new();
            self.root_id = None;
            self.click_listeners.clear();
            self.wasm_base_id = None;
            self.last_seen_id = None;
            self.id_map.clear();
            self.next_host_id = 0;
        }
    }
    
    /// Detect if WASM has restarted by checking if we're setting a new root
    /// after already having one with a significant ID gap
    fn detect_wasm_restart(&mut self, new_id: u32) {
        if let Some(last) = self.last_seen_id {
            // If new_id is not sequential (gap > 1), it indicates WASM restart
            // since the counter continued from previous session
            if new_id > last && new_id - last > 1 {
                log::info!("WASM restart detected: last_id={}, new_id={}, new session starts at {}", 
                    last, new_id, new_id);
                self.wasm_base_id = Some(new_id);
            }
        } else {
            // First node ever - set base_id
            self.wasm_base_id = Some(new_id);
            log::info!("WASM base_id set to {}", new_id);
        }
        self.last_seen_id = Some(new_id);
    }
    
    /// Get the Host ID for a WASM ID (for already mapped IDs)
    fn get_host_id(&self, wasm_id: u32) -> Option<u32> {
        self.id_map.get(&wasm_id).copied()
    }
    
    /// Resolve a WASM ID to a Host ID for node operations
    /// This should be called at the beginning of all public methods that take an ID
    fn resolve_id(&self, wasm_id: u32) -> u32 {
        self.get_host_id(wasm_id).unwrap_or_else(|| {
            // If not mapped, return the original ID (backward compatibility)
            wasm_id
        })
    }
    
    /// Map a WASM node ID to a Host node ID
    /// This handles hot restart where WASM IDs may jump (e.g., 0-199, then 200-399)
    fn map_wasm_id(&mut self, wasm_id: u32) -> u32 {
        // Check if this is a new session (ID jump detected)
        // Only consider it a restart if the jump is significant (> 1000)
        // This avoids false positives during normal batch operations within a transaction
        let is_new_session = if let Some(last) = self.last_seen_id {
            // Gap detected: WASM restarted with continued counter
            // Threshold: jump > 1000 indicates restart (normal app jumps are smaller)
            wasm_id > last && wasm_id > last + 1000
        } else {
            true // First session
        };
        
        if is_new_session {
            if self.root_id.is_some() {
                log::info!("WASM hot restart detected: WASM id={}, previous last_id={} (jump > 1000)", 
                    wasm_id, self.last_seen_id.unwrap());
            }
            // Reset for new session
            self.wasm_base_id = Some(wasm_id);
            self.id_map.clear();
            self.next_host_id = 0;
        }
        
        self.last_seen_id = Some(wasm_id);
        
        // Map WASM ID to Host ID
        if let Some(&host_id) = self.id_map.get(&wasm_id) {
            host_id
        } else {
            let host_id = self.next_host_id;
            self.id_map.insert(wasm_id, host_id);
            self.next_host_id += 1;
            
            // Log first few mappings
            if host_id < 3 {
                log::info!("ID mapping: WASM id={} -> Host id={}", wasm_id, host_id);
            }
            
            host_id
        }
    }
    
    pub fn create_node(&mut self, wasm_id: u32) {
        let host_id = self.map_wasm_id(wasm_id);
        
        // Set root if this is the first node
        if self.root_id.is_none() {
            self.root_id = Some(host_id);
            log::info!("Root node set to Host id={} (WASM id={})", host_id, wasm_id);
        }
        
        let exists = self.nodes.contains_key(&host_id);
        let taffy_node = self.taffy.new_leaf(Style::default()).unwrap();
        if exists {
            log::debug!("create_node: REPLACING existing node Host id={} (WASM id={})", host_id, wasm_id);
        }
        
        self.nodes.insert(host_id, ViewNode { 
            taffy_node, 
            color: Color::WHITE, 
            children: vec![], 
            z_index: 0, 
            label: String::new(), 
            text: String::new(),
            font_size: 16.0,
            font_family: String::new(),
            font_weight: 400,
            border_radius: 0.0, 
            role: Role::None, 
            view_type: ViewType::Container, 
            has_click: false, 
            padding: (0.0, 0.0, 0.0, 0.0),
            dirty_fields: 0,
        });
    }
    
    pub fn create_text_node(&mut self, wasm_id: u32) {
        self.create_node(wasm_id);
        // create_node handles the ID mapping
        let host_id = self.get_host_id(wasm_id).unwrap_or(wasm_id);
        self.set_view_type(host_id, 1); // ViewType::Text
    }

    pub fn set_font_family(&mut self, wasm_id: u32, family: String) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) { node.font_family = family; }
    }

    pub fn set_font_weight(&mut self, wasm_id: u32, weight: u16) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) { node.font_weight = weight; }
    }

    pub fn set_color_rgba(&mut self, wasm_id: u32, r: u8, g: u8, b: u8, a: u8) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) { node.color = Color::from_rgba8(r, g, b, a); }
    }

    pub fn set_view_type(&mut self, wasm_id: u32, vt: u32) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) { 
            node.view_type = match vt { 1 => ViewType::Text, 2 => ViewType::Button, _ => ViewType::Container }; 
        } 
    }
    
    pub fn set_text(&mut self, wasm_id: u32, text: String) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) { 
            node.text = text; 
        }
    }
    
    pub fn set_font_size(&mut self, wasm_id: u32, size: f32) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) { node.font_size = size; } 
    }
    
    pub fn set_color(&mut self, wasm_id: u32, r: u8, g: u8, b: u8) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) { node.color = Color::from_rgb8(r, g, b); } 
    }
    
    pub fn set_width(&mut self, wasm_id: u32, dt: u32, v: f32) { 
        let id = self.resolve_id(wasm_id);
        log::warn!("[Layout] set_width: wasm_id={} -> host_id={}, dt={}, v={}", wasm_id, id, dt, v);
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            let width = match dt { 1 => taffy::style::Dimension::length(v), 2 => taffy::style::Dimension::percent(v / 100.0), _ => taffy::style::Dimension::auto() };
            s.size.width = width;
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
            log::warn!("[Layout] set_width applied: host_id={} width={:?}", id, width);
        } else {
            log::warn!("[Layout] set_width: node not found! host_id={}", id);
        }
    }
    
    pub fn set_height(&mut self, wasm_id: u32, dt: u32, v: f32) { 
        let id = self.resolve_id(wasm_id);
        log::warn!("[Layout] set_height: wasm_id={} -> host_id={}, dt={}, v={}", wasm_id, id, dt, v);
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            let height = match dt { 1 => taffy::style::Dimension::length(v), 2 => taffy::style::Dimension::percent(v / 100.0), _ => taffy::style::Dimension::auto() };
            s.size.height = height;
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
            log::warn!("[Layout] set_height applied: host_id={} height={:?}", id, height);
        } else {
            log::warn!("[Layout] set_height: node not found! host_id={}", id);
        }
    }
    
    pub fn set_flex_direction(&mut self, wasm_id: u32, dir: u32) { 
        let id = self.resolve_id(wasm_id);
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
    
    pub fn set_justify_content(&mut self, wasm_id: u32, j: u32) { 
        let id = self.resolve_id(wasm_id);
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
    
    pub fn set_align_items(&mut self, wasm_id: u32, a: u32) { 
        let id = self.resolve_id(wasm_id);
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
    
    pub fn set_flex_wrap(&mut self, wasm_id: u32, w: u32) { 
        let id = self.resolve_id(wasm_id);
        log::info!("[Layout] set_flex_wrap: wasm_id={} -> host_id={}", wasm_id, id);
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            let wrap_value = match w { 
                1 => taffy::prelude::FlexWrap::Wrap, 
                2 => taffy::prelude::FlexWrap::WrapReverse, 
                _ => taffy::prelude::FlexWrap::NoWrap 
            }; 
            s.flex_wrap = wrap_value; 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
            log::info!("[Layout] set_flex_wrap applied: host_id={} wrap={:?}", id, wrap_value);
        } else {
            log::warn!("[Layout] set_flex_wrap: node not found! host_id={}", id);
        }
    }
    
    pub fn set_align_content(&mut self, wasm_id: u32, ac: u32) { 
        let id = self.resolve_id(wasm_id);
        log::info!("[Layout] set_align_content: wasm_id={} -> host_id={}", wasm_id, id);
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            let ac_value = Some(match ac { 
                1 => taffy::prelude::AlignContent::Center, 
                2 => taffy::prelude::AlignContent::FlexEnd, 
                3 => taffy::prelude::AlignContent::Stretch, 
                4 => taffy::prelude::AlignContent::SpaceBetween, 
                5 => taffy::prelude::AlignContent::SpaceAround, 
                6 => taffy::prelude::AlignContent::SpaceEvenly, 
                _ => taffy::prelude::AlignContent::FlexStart 
            }); 
            s.align_content = ac_value; 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
            log::info!("[Layout] set_align_content applied: host_id={} align_content={:?}", id, ac_value);
        } else {
            log::warn!("[Layout] set_align_content: node not found! host_id={}", id);
        }
    }
    
    pub fn set_flex_grow(&mut self, wasm_id: u32, grow: f32) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.flex_grow = grow; 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn set_z_index(&mut self, wasm_id: u32, z: i32) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) { node.z_index = z; } 
    }
    
    pub fn set_border_radius(&mut self, wasm_id: u32, r: f32) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) { node.border_radius = r; } 
    }
    
    pub fn set_padding(&mut self, wasm_id: u32, t: f32, r: f32, b: f32, l: f32) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.padding.top = LengthPercentage::length(t).into(); 
            s.padding.right = LengthPercentage::length(r).into(); 
            s.padding.bottom = LengthPercentage::length(b).into(); 
            s.padding.left = LengthPercentage::length(l).into(); 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        } 
    }
    
    pub fn attach_click(&mut self, wasm_id: u32) { 
        let id = self.resolve_id(wasm_id);
        self.click_listeners.push(id); 
        if let Some(node) = self.nodes.get_mut(&id) { node.has_click = true; } 
    }
    
    pub fn add_child(&mut self, wasm_pid: u32, wasm_cid: u32) { 
        // Map WASM IDs to Host IDs
        let host_pid = self.get_host_id(wasm_pid)
            .unwrap_or_else(|| {
                log::warn!("add_child: parent WASM id={} not found in mapping, using 0", wasm_pid);
                0
            });
        let host_cid = self.get_host_id(wasm_cid)
            .unwrap_or_else(|| {
                log::warn!("add_child: child WASM id={} not found in mapping, using 0", wasm_cid);
                0
            });
        
        // Debug: Log first 40 add_child calls
        static mut ADDCOUNT: i32 = 0;
        unsafe {
            ADDCOUNT += 1;
            if ADDCOUNT <= 40 {
                log::warn!("[ADDCHILD-TRACE] #{}: wasm({}->{}) -> host({}->{})", 
                    ADDCOUNT, wasm_pid, wasm_cid, host_pid, host_cid);
            }
        }
        
        let c_tn = self.nodes.get(&host_cid).map(|n| n.taffy_node); 
        let p_tn = self.nodes.get(&host_pid).map(|n| n.taffy_node); 
        if let (Some(ptn), Some(ctn)) = (p_tn, c_tn) { 
            if let Some(parent) = self.nodes.get_mut(&host_pid) {
                if parent.children.contains(&host_cid) {
                    log::debug!("add_child: parent {} already has child {}, skipping", host_pid, host_cid);
                    return;
                }
                log::debug!("add_child: parent {} adding child {} (WASM: {} -> {}, {} -> {})", 
                    host_pid, host_cid, wasm_pid, host_pid, wasm_cid, host_cid);
                parent.children.push(host_cid);
                self.taffy.add_child(ptn, ctn).unwrap();
            } else {
                log::warn!("add_child: parent {} not found", host_pid);
            }
        } else {
            log::warn!("add_child: parent {} (WASM {}) or child {} (WASM {}) taffy node not found", 
                host_pid, wasm_pid, host_cid, wasm_cid);
        }
    }
    
    pub fn set_font_data(&mut self, data: Vec<u8>) { self.font_data = Some(data); }
    
    /// Mark node fields as dirty for command deduplication
    pub fn set_node_dirty(&mut self, wasm_id: u32, fields: u8) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.dirty_fields |= fields;
        }
    }
    
    /// Check if a node has dirty fields
    pub fn is_node_dirty(&self, wasm_id: u32, field_mask: u8) -> bool {
        let id = self.resolve_id(wasm_id);
        self.nodes.get(&id)
            .map(|n| n.dirty_fields & field_mask != 0)
            .unwrap_or(false)
    }
    
    /// Clear dirty fields for all nodes (called after frame render)
    pub fn clear_all_dirty(&mut self) {
        for node in self.nodes.values_mut() {
            node.dirty_fields = 0;
        }
    }
}
