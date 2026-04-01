// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::HashMap;
use taffy::prelude::*;
use peniko::Color;
use crate::types::{ViewType, Role};
use crate::{NodeHandle, MAX_CAPACITY, INITIAL_CAPACITY};

pub struct ViewNode { 
    pub taffy_node: NodeId, 
    pub color: Color, 
    pub children: Vec<u32>, 
    /// Parent node ID (0 means no parent/root)
    pub parent_id: u32,
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
    /// Last measured size for detecting size changes that require relayout
    pub last_measured_size: (f32, f32),
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
    
    // === 代际ID支持 ===
    /// 当前容量（动态扩容，初始为 INITIAL_CAPACITY）
    capacity: usize,
    /// 每个槽位的代际计数器（防止 Stale ID）
    generations: [u32; MAX_CAPACITY],
    /// 空闲槽位列表（回收的ID）
    free_ids: Vec<u32>,
    /// 活跃节点映射: WASM ID -> NodeHandle
    active_handles: HashMap<u32, NodeHandle>,
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
            capacity: INITIAL_CAPACITY,
            generations: [0; MAX_CAPACITY],
            free_ids: Vec::new(),
            active_handles: HashMap::new(),
        } 
    }

    /// Clear all state - used when WASM is reloaded (hot restart)
    pub fn clear(&mut self) {
        let node_count = self.nodes.len();
        if node_count > 0 {
            self.nodes.clear();
            self.taffy = TaffyTree::new();
            self.root_id = None;
            self.click_listeners.clear();
            self.wasm_base_id = None;
            self.last_seen_id = None;
            self.id_map.clear();
            self.next_host_id = 0;
            self.capacity = INITIAL_CAPACITY;
            self.generations = [0; MAX_CAPACITY];
            self.free_ids.clear();
            self.active_handles.clear();
        }
    }
    
    #[allow(dead_code)]
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
            

            
            host_id
        }
    }
    
    pub fn create_node(&mut self, wasm_id: u32) {
        let host_id = self.map_wasm_id(wasm_id);
        
        // Set root if this is the first node
        if self.root_id.is_none() {
            self.root_id = Some(host_id);

        }
        
        let exists = self.nodes.contains_key(&host_id);
        let taffy_node = self.taffy.new_leaf(Style::default()).unwrap();
        if exists {

        }
        
        self.nodes.insert(host_id, ViewNode { 
            taffy_node, 
            color: Color::WHITE, 
            children: vec![], 
            parent_id: 0,
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
            last_measured_size: (0.0, 0.0),
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
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            let width = match dt { 1 => taffy::style::Dimension::length(v), 2 => taffy::style::Dimension::percent(v / 100.0), _ => taffy::style::Dimension::auto() };
            s.size.width = width;
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        }
    }
    
    pub fn set_height(&mut self, wasm_id: u32, dt: u32, v: f32) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            let height = match dt { 1 => taffy::style::Dimension::length(v), 2 => taffy::style::Dimension::percent(v / 100.0), _ => taffy::style::Dimension::auto() };
            s.size.height = height;
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
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
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.flex_wrap = match w { 
                1 => taffy::prelude::FlexWrap::Wrap, 
                2 => taffy::prelude::FlexWrap::WrapReverse, 
                _ => taffy::prelude::FlexWrap::NoWrap 
            }; 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
        }
    }
    
    pub fn set_align_content(&mut self, wasm_id: u32, ac: u32) { 
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get(&id) { 
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); 
            s.align_content = Some(match ac { 
                1 => taffy::prelude::AlignContent::Center, 
                2 => taffy::prelude::AlignContent::FlexEnd, 
                3 => taffy::prelude::AlignContent::Stretch, 
                4 => taffy::prelude::AlignContent::SpaceBetween, 
                5 => taffy::prelude::AlignContent::SpaceAround, 
                6 => taffy::prelude::AlignContent::SpaceEvenly, 
                _ => taffy::prelude::AlignContent::FlexStart 
            }); 
            self.taffy.set_style(node.taffy_node, s).unwrap(); 
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
        let host_pid = self.get_host_id(wasm_pid).unwrap_or(0);
        let host_cid = self.get_host_id(wasm_cid).unwrap_or(0);
        
        let c_tn = self.nodes.get(&host_cid).map(|n| n.taffy_node);
        let p_tn = self.nodes.get(&host_pid).map(|n| n.taffy_node);
        if let (Some(ptn), Some(ctn)) = (p_tn, c_tn) {
            if let Some(parent) = self.nodes.get_mut(&host_pid) {
                if !parent.children.contains(&host_cid) {
                    parent.children.push(host_cid);
                    // Update child's parent reference
                    if let Some(child) = self.nodes.get_mut(&host_cid) {
                        child.parent_id = host_pid;
                    }
                    let _ = self.taffy.add_child(ptn, ctn);
                }
            }
        }
    }
    
    /// Get parent node ID for a given child node ID (0 means no parent)
    pub fn get_parent(&self, node_id: u32) -> u32 {
        self.nodes.get(&node_id).map(|n| n.parent_id).unwrap_or(0)
    }
    
    /// Collect all ancestor node IDs (from immediate parent to root)
    pub fn get_ancestors(&self, node_id: u32) -> Vec<u32> {
        let mut ancestors = Vec::new();
        let mut current = node_id;
        while current != 0 {
            let parent_id = self.get_parent(current);
            if parent_id == 0 { break; }
            ancestors.push(parent_id);
            current = parent_id;
        }
        ancestors
    }
    
    /// Mark a node as dirty by re-setting its Taffy style
    /// Taffy's set_style automatically calls mark_dirty which recursively marks all ancestors
    pub fn mark_dirty(&mut self, node_id: u32) {
        if let Some(node) = self.nodes.get(&node_id) {
            if let Ok(style) = self.taffy.style(node.taffy_node) {
                let new_style = style.clone();
                let _ = self.taffy.set_style(node.taffy_node, new_style);
            }
        }
    }
    
    /// Get layout result for a node (for LayoutRegistry)
    pub fn get_layout(&self, wasm_id: u32) -> Option<(f32, f32, f32, f32)> {
        let id = self.resolve_id(wasm_id);
        self.nodes.get(&id).and_then(|node| {
            self.taffy.layout(node.taffy_node).ok().map(|l| {
                (l.location.x, l.location.y, l.size.width, l.size.height)
            })
        })
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
    
    /// Measure text nodes and update their Taffy styles before layout
    /// This is a simplified text measurement - real implementation should use a font library
    pub fn measure_text_nodes(&mut self) {
        for (_id, node) in self.nodes.iter_mut() {
            if node.view_type == ViewType::Text && !node.text.is_empty() {
                // Simplified text measurement: estimate based on character count and font size
                // Real implementation should use cosmic-text or similar
                let avg_char_width = node.font_size * 0.6; // rough estimate
                let estimated_width = node.text.len() as f32 * avg_char_width;
                let estimated_height = node.font_size * 1.2; // line height
                
                // Update Taffy style with measured size
                if let Ok(style) = self.taffy.style(node.taffy_node) {
                    let mut new_style = style.clone();
                    new_style.size.width = taffy::prelude::Dimension::length(estimated_width);
                    new_style.size.height = taffy::prelude::Dimension::length(estimated_height);
                    let _ = self.taffy.set_style(node.taffy_node, new_style);
                }
            }
        }
    }
    
    // === 代际ID支持 ===
    
    /// 分配一个新的节点ID（优先使用回收的ID）
    fn allocate_id(&mut self) -> u32 {
        // 优先使用回收的ID
        if let Some(id) = self.free_ids.pop() {
            return id;
        }
        
        // 否则分配新ID
        let id = self.next_host_id;
        self.next_host_id += 1;
        id
    }
    
    /// 创建节点并返回 NodeHandle（代际ID版本）
    pub fn create_node_with_handle(&mut self, wasm_id: u32) -> Option<NodeHandle> {
        let slot = self.allocate_id();
        
        // 检查是否超出容量
        if slot as usize >= self.capacity {
            // 尝试扩容（简化版，实际应调用 expand_capacity）
            if !self.try_expand_capacity() {
                log::warn!("Node capacity exceeded: {}/{}", slot, self.capacity);
                return None;
            }
        }
        
        let generation = self.generations[slot as usize];
        let handle = NodeHandle::new(slot, generation);
        
        // 创建 Taffy 节点
        let taffy_node = self.taffy.new_leaf(Style::default()).ok()?;
        
        // 插入节点
        self.nodes.insert(slot, ViewNode {
            taffy_node,
            color: Color::WHITE,
            children: vec![],
            parent_id: 0,
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
            last_measured_size: (0.0, 0.0),
        });
        
        // 记录映射
        self.id_map.insert(wasm_id, slot);
        self.active_handles.insert(wasm_id, handle);
        
        // 设置根节点
        if self.root_id.is_none() {
            self.root_id = Some(slot);
        }
        
        Some(handle)
    }
    
    /// 验证 NodeHandle 是否有效
    pub fn verify_handle(&self, handle: NodeHandle) -> bool {
        if !handle.is_valid() {
            return false;
        }
        let slot = handle.slot as usize;
        if slot >= self.capacity {
            return false;
        }
        // 检查代际是否匹配
        self.generations[slot] == handle.generation && self.nodes.contains_key(&handle.slot)
    }
    
    /// 获取 NodeHandle 对应的节点
    pub fn get_node_by_handle(&self, handle: NodeHandle) -> Option<&ViewNode> {
        if self.verify_handle(handle) {
            self.nodes.get(&handle.slot)
        } else {
            None
        }
    }
    
    /// 获取 NodeHandle 对应的节点（可变）
    pub fn get_node_by_handle_mut(&mut self, handle: NodeHandle) -> Option<&mut ViewNode> {
        if self.verify_handle(handle) {
            self.nodes.get_mut(&handle.slot)
        } else {
            None
        }
    }
    
    /// 删除节点并回收ID（增加代际）
    pub fn remove_node_with_handle(&mut self, handle: NodeHandle) -> bool {
        if !self.verify_handle(handle) {
            return false;
        }
        
        let slot = handle.slot;
        
        // 从 Taffy 中移除
        if let Some(node) = self.nodes.get(&slot) {
            let _ = self.taffy.remove(node.taffy_node);
        }
        
        // 从 nodes 中移除
        self.nodes.remove(&slot);
        
        // 清理映射
        self.active_handles.retain(|_, h| h.slot != slot);
        
        // 增加代际（防止 Stale ID）
        let slot_idx = slot as usize;
        if slot_idx < MAX_CAPACITY {
            self.generations[slot_idx] = self.generations[slot_idx].wrapping_add(1);
        }
        
        // 回收ID
        self.free_ids.push(slot);
        
        // 清理子节点的 parent_id
        for node in self.nodes.values_mut() {
            if node.parent_id == slot {
                node.parent_id = 0;
            }
        }
        
        true
    }
    
    /// 尝试扩容（简化版）
    fn try_expand_capacity(&mut self) -> bool {
        // 找到下一个容量档位
        for &level in crate::CAPACITY_LEVELS.iter() {
            if level > self.capacity && level <= MAX_CAPACITY {
                self.capacity = level;
                log::info!("Node capacity expanded to {}", level);
                return true;
            }
        }
        false
    }
    
    /// 获取当前容量
    pub fn get_capacity(&self) -> usize {
        self.capacity
    }
    
    /// 获取代际数组（用于同步到 SharedBuffer）
    pub fn get_generations(&self) -> &[u32; MAX_CAPACITY] {
        &self.generations
    }
}
