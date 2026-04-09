// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::types::{Role, ViewType};
use crate::{NodeHandle, INITIAL_CAPACITY, MAX_CAPACITY};
use peniko::Color;
use std::collections::HashMap;
use taffy::prelude::*;

/// Text alignment options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// Left/start aligned
    #[default]
    Start = 0,
    /// Center aligned
    Center = 1,
    /// Right/end aligned
    End = 2,
    /// Justified
    Justified = 3,
}

impl TextAlign {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => TextAlign::Center,
            2 => TextAlign::End,
            3 => TextAlign::Justified,
            _ => TextAlign::Start,
        }
    }
}

/// TextInput input types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum InputType {
    #[default]
    Text = 0,
    Password = 1,
    Number = 2,
    Email = 3,
    Phone = 4,
    URL = 5,
    Decimal = 6,
    Multiline = 7,
}

impl InputType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => InputType::Password,
            2 => InputType::Number,
            3 => InputType::Email,
            4 => InputType::Phone,
            5 => InputType::URL,
            6 => InputType::Decimal,
            7 => InputType::Multiline,
            _ => InputType::Text,
        }
    }
}

/// Return key types for virtual keyboard
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ReturnKeyType {
    #[default]
    Default = 0,
    Go = 1,
    Google = 2,
    Join = 3,
    Next = 4,
    Route = 5,
    Search = 6,
    Send = 7,
    Done = 8,
    EmergencyCall = 9,
    Continue = 10,
}

impl ReturnKeyType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => ReturnKeyType::Go,
            2 => ReturnKeyType::Google,
            3 => ReturnKeyType::Join,
            4 => ReturnKeyType::Next,
            5 => ReturnKeyType::Route,
            6 => ReturnKeyType::Search,
            7 => ReturnKeyType::Send,
            8 => ReturnKeyType::Done,
            9 => ReturnKeyType::EmergencyCall,
            10 => ReturnKeyType::Continue,
            _ => ReturnKeyType::Default,
        }
    }
}

/// Performance optimization configuration for TextInput
#[derive(Debug, Clone, Copy)]
pub struct TextInputPerfConfig {
    /// Large text threshold (bytes). Above this, optimizations kick in.
    pub large_text_threshold: usize,
    /// Maximum allowed text length (0 = unlimited)
    pub max_text_length: usize,
    /// Whether to enable viewport-based rendering for large text
    pub viewport_optimization: bool,
    /// Maximum height in lines for single-line inputs (0 = unlimited)
    pub max_visible_lines: usize,
}

impl Default for TextInputPerfConfig {
    fn default() -> Self {
        Self {
            large_text_threshold: 4096,   // 4KB threshold
            max_text_length: 1024 * 1024, // 1MB max
            viewport_optimization: true,
            max_visible_lines: 0, // 0 = unlimited
        }
    }
}

/// TextInput state for managing text input nodes
#[derive(Debug, Clone)]
pub struct TextInputState {
    /// Current text content
    pub text: String,
    /// Placeholder text (shown when empty)
    pub placeholder: String,
    /// IME composing text (in-progress input for CJK)
    pub composing_text: String,
    /// Cursor position (UTF-8 byte index)
    pub cursor_pos: usize,
    /// Selection start (if != cursor_pos, text is selected)
    pub selection_start: usize,
    /// Input type
    pub input_type: InputType,
    /// Return key type
    pub return_key_type: ReturnKeyType,
    /// Whether the input is currently focused
    pub focused: bool,
    /// Whether the input is enabled
    pub enabled: bool,
    /// Whether the input is read-only
    pub read_only: bool,
    /// Maximum text length (0 = unlimited)
    pub max_length: u32,
    /// Whether autocorrect is enabled
    pub auto_correct: bool,
    /// Whether the input is secure (password mode)
    pub secure: bool,
    /// Cursor visibility (for blinking)
    pub cursor_visible: bool,
    /// Last blink timestamp
    pub last_blink_time: u64,
    /// Horizontal scroll offset
    pub scroll_offset_x: f32,
    /// Vertical scroll offset
    pub scroll_offset_y: f32,
    // === Phase 5: Performance optimizations ===
    /// Generation counter for dirty tracking (incremented on each change)
    pub generation: u64,
    /// Performance configuration
    pub perf_config: TextInputPerfConfig,
    /// Cached text hash for fast comparison
    text_hash: u64,
    /// Whether this input is in "large text" mode (optimizations active)
    pub is_large_text: bool,
}

/// Calculate hash for text comparison
fn hash_text(text: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl Default for TextInputState {
    fn default() -> Self {
        Self {
            text: String::new(),
            placeholder: String::new(),
            composing_text: String::new(),
            cursor_pos: 0,
            selection_start: 0,
            input_type: InputType::Text,
            return_key_type: ReturnKeyType::Default,
            focused: false,
            enabled: true,
            read_only: false,
            max_length: 0,
            auto_correct: true,
            secure: false,
            cursor_visible: true,
            last_blink_time: 0,
            scroll_offset_x: 0.0,
            scroll_offset_y: 0.0,
            // Phase 5: Performance optimizations
            generation: 0,
            perf_config: TextInputPerfConfig::default(),
            text_hash: 0,
            is_large_text: false,
        }
    }
}

impl TextInputState {
    /// Check if there's an active selection
    pub fn has_selection(&self) -> bool {
        self.selection_start != self.cursor_pos
    }

    /// Get selection range (start, end) ordered
    pub fn selection_range(&self) -> (usize, usize) {
        if self.selection_start < self.cursor_pos {
            (self.selection_start, self.cursor_pos)
        } else {
            (self.cursor_pos, self.selection_start)
        }
    }

    /// Clear selection (collapse to cursor position)
    pub fn clear_selection(&mut self) {
        self.selection_start = self.cursor_pos;
    }

    /// Select all text
    pub fn select_all(&mut self) {
        self.selection_start = 0;
        self.cursor_pos = self.text.len();
    }

    /// Insert text at cursor position
    pub fn insert_text(&mut self, text: &str) {
        if self.read_only {
            return;
        }

        // Delete selected text if any
        if self.has_selection() {
            let (start, end) = self.selection_range();
            self.text.replace_range(start..end, "");
            self.cursor_pos = start;
            self.selection_start = start;
        }

        // Check max length
        if self.max_length > 0 {
            let remaining = self.max_length as usize - self.text.len();
            let text_to_insert = &text[..text.len().min(remaining)];
            self.text.insert_str(self.cursor_pos, text_to_insert);
            self.cursor_pos += text_to_insert.len();
        } else {
            self.text.insert_str(self.cursor_pos, text);
            self.cursor_pos += text.len();
        }
        self.selection_start = self.cursor_pos;
        self.mark_changed();
    }

    /// Delete selected text or character before cursor
    pub fn backspace(&mut self) {
        if self.read_only {
            return;
        }

        if self.has_selection() {
            let (start, end) = self.selection_range();
            self.text.replace_range(start..end, "");
            self.cursor_pos = start;
            self.selection_start = start;
            self.mark_changed();
        } else if self.cursor_pos > 0 {
            let char_len = self.text[..self.cursor_pos]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.cursor_pos -= char_len;
            self.text
                .replace_range(self.cursor_pos..self.cursor_pos + char_len, "");
            self.selection_start = self.cursor_pos;
            self.mark_changed();
        }
    }

    /// Delete character after cursor
    pub fn delete(&mut self) {
        if self.read_only {
            return;
        }

        if self.has_selection() {
            let (start, end) = self.selection_range();
            self.text.replace_range(start..end, "");
            self.cursor_pos = start;
            self.selection_start = start;
            self.mark_changed();
        } else if self.cursor_pos < self.text.len() {
            let char_len = self.text[self.cursor_pos..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.text
                .replace_range(self.cursor_pos..self.cursor_pos + char_len, "");
            self.mark_changed();
        }
    }

    /// Get selected text
    pub fn selected_text(&self) -> Option<String> {
        if !self.has_selection() {
            return None;
        }
        let (start, end) = self.selection_range();
        Some(self.text[start..end].to_string())
    }

    /// Set text and reset cursor
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor_pos = self.text.len();
        self.selection_start = self.cursor_pos;
        self.mark_changed();
    }

    // === IME Composition ===

    /// Check if currently composing (IME input in progress)
    pub fn is_composing(&self) -> bool {
        !self.composing_text.is_empty()
    }

    /// Get the current composition range (if composing)
    pub fn composition_range(&self) -> Option<(usize, usize)> {
        if self.composing_text.is_empty() {
            None
        } else {
            // Composition happens at cursor position
            let start = self.cursor_pos;
            let end = start + self.composing_text.len();
            Some((start, end))
        }
    }

    /// Start or update IME composition
    pub fn set_composing_text(&mut self, text: String) {
        // Clear any previous composition
        self.clear_composing_text();
        self.composing_text = text;
    }

    /// Clear composing text without committing
    pub fn clear_composing_text(&mut self) {
        self.composing_text.clear();
    }

    /// Commit composing text to the actual text
    pub fn commit_composing_text(&mut self) {
        if !self.composing_text.is_empty() {
            // Insert composing text at cursor position
            self.text.insert_str(self.cursor_pos, &self.composing_text);
            self.cursor_pos += self.composing_text.len();
            self.selection_start = self.cursor_pos;
            self.composing_text.clear();
            self.mark_changed();
        }
    }

    // === Phase 5: Performance optimization methods ===

    /// Mark state as changed (increments generation and updates hash)
    fn mark_changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.text_hash = hash_text(&self.text);
        self.check_large_text();
    }

    /// Check if text exceeds large text threshold
    fn check_large_text(&mut self) {
        self.is_large_text = self.text.len() > self.perf_config.large_text_threshold;
    }

    /// Set text with change tracking
    pub fn set_text_tracked(&mut self, text: String) {
        let new_hash = hash_text(&text);
        if self.text_hash != new_hash {
            self.text = text;
            self.cursor_pos = self.text.len().min(self.cursor_pos);
            self.selection_start = self.cursor_pos;
            self.mark_changed();
        }
    }

    /// Check if text has changed compared to a previous hash
    pub fn has_changed_since(&self, prev_hash: u64) -> bool {
        self.text_hash != prev_hash
    }

    /// Get current text hash for comparison
    pub fn text_hash(&self) -> u64 {
        self.text_hash
    }

    /// Truncate text if it exceeds maximum length
    pub fn enforce_max_length(&mut self) {
        let max = self.perf_config.max_text_length;
        if max > 0 && self.text.len() > max {
            // Find valid UTF-8 boundary
            let mut trunc_len = max;
            while trunc_len > 0 && !self.text.is_char_boundary(trunc_len) {
                trunc_len -= 1;
            }
            self.text.truncate(trunc_len);
            self.cursor_pos = self.cursor_pos.min(trunc_len);
            self.selection_start = self.selection_start.min(trunc_len);
            self.mark_changed();
        }
    }

    /// Get visible text range for viewport optimization (large text mode)
    /// Returns (start_byte, end_byte) for the visible portion
    pub fn visible_range(
        &self,
        viewport_start: f32,
        viewport_height: f32,
        line_height: f32,
    ) -> (usize, usize) {
        if !self.is_large_text || !self.perf_config.viewport_optimization {
            return (0, self.text.len());
        }

        // Estimate lines based on viewport
        let start_line = (viewport_start / line_height.max(1.0)) as usize;
        let visible_lines = (viewport_height / line_height.max(1.0)) as usize;
        let end_line = start_line + visible_lines + 2; // Buffer of 2 lines

        // Convert line numbers to byte positions (approximate)
        // This is a simple approximation - for precise positioning,
        // the renderer would need to provide exact byte positions
        let chars_per_line = 80; // Approximate
        let start_char = start_line.saturating_mul(chars_per_line);
        let end_char = (end_line * chars_per_line).min(self.text.len());

        // Find valid UTF-8 boundaries
        let start_byte = self
            .text
            .char_indices()
            .nth(start_char)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let end_byte = self
            .text
            .char_indices()
            .nth(end_char)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());

        (start_byte, end_byte)
    }
}

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
    pub text_align: TextAlign,
    pub border_radius: f32,
    /// Opacity (0.0 - 1.0, 1.0 = fully opaque)
    pub opacity: f32,
    /// Clip children to bounds
    pub clip_to_bounds: bool,
    /// Shadow offset X
    pub shadow_offset_x: f32,
    /// Shadow offset Y
    pub shadow_offset_y: f32,
    /// Shadow blur radius
    pub shadow_blur: f32,
    /// Shadow color (RGBA)
    pub shadow_color: u32,
    /// Blur radius for the node itself
    pub blur_radius: f32,
    /// Border stroke width
    pub border_width: f32,
    /// Border color (RGBA)
    pub border_color: u32,
    /// Position offset for absolute positioning
    pub position_x: f32,
    pub position_y: f32,
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

    // === SharedBuffer 同步 ===
    /// SharedBuffer 指针（用于 Render 线程同步布局结果）
    shared_buffer_ptr: Option<*mut crate::SharedBuffer>,
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
            shared_buffer_ptr: None,
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
            // 注意：不清除 shared_buffer_ptr，因为缓冲区通常不变
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
                log::info!(
                    "WASM restart detected: last_id={}, new_id={}, new session starts at {}",
                    last,
                    new_id,
                    new_id
                );
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
    pub fn resolve_id(&self, wasm_id: u32) -> u32 {
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
        if exists {}

        self.nodes.insert(
            host_id,
            ViewNode {
                taffy_node,
                color: Color::TRANSPARENT,
                children: Vec::new(),
                parent_id: 0,
                z_index: 0,
                label: String::new(),
                text: String::new(),
                font_size: 16.0,
                font_family: String::new(),
                font_weight: 400,
                text_align: TextAlign::Start,
                border_radius: 0.0,
                opacity: 1.0,
                clip_to_bounds: false,
                shadow_offset_x: 0.0,
                shadow_offset_y: 0.0,
                shadow_blur: 0.0,
                shadow_color: 0xFF000000, // Black shadow default
                blur_radius: 0.0,
                border_width: 0.0,
                border_color: 0xFF000000, // Black default
                position_x: 0.0,
                position_y: 0.0,
                role: Role::None,
                view_type: ViewType::Container,
                has_click: false,
                padding: (0.0, 0.0, 0.0, 0.0),
                dirty_fields: 0,
                last_measured_size: (0.0, 0.0),
            },
        );
    }

    pub fn create_text_node(&mut self, wasm_id: u32) {
        self.create_node(wasm_id);
        // create_node handles the ID mapping
        let host_id = self.get_host_id(wasm_id).unwrap_or(wasm_id);
        self.set_view_type(host_id, 1); // ViewType::Text
    }

    pub fn set_font_family(&mut self, wasm_id: u32, family: String) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.font_family = family;
        }
    }

    pub fn set_font_weight(&mut self, wasm_id: u32, weight: u16) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.font_weight = weight;
        }
    }

    pub fn set_text_align(&mut self, wasm_id: u32, align: u8) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.text_align = TextAlign::from_u8(align);
        }
    }

    pub fn set_color_rgba(&mut self, wasm_id: u32, r: u8, g: u8, b: u8, a: u8) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.color = Color::from_rgba8(r, g, b, a);
        }
    }

    pub fn set_view_type(&mut self, wasm_id: u32, vt: u32) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.view_type = match vt {
                1 => ViewType::Text,
                2 => ViewType::Button,
                3 => ViewType::Image,
                4 => ViewType::Input,
                _ => ViewType::Container,
            };
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
        if let Some(node) = self.nodes.get_mut(&id) {
            node.font_size = size;
        }
    }

    pub fn set_color(&mut self, wasm_id: u32, r: u8, g: u8, b: u8) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.color = Color::from_rgb8(r, g, b);
        }
    }

    pub fn set_width(&mut self, wasm_id: u32, dt: u32, v: f32) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get(&id) {
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone();
            let width = match dt {
                1 => taffy::style::Dimension::length(v),
                2 => taffy::style::Dimension::percent(v / 100.0),
                _ => taffy::style::Dimension::auto(),
            };
            s.size.width = width;
            self.taffy.set_style(node.taffy_node, s).unwrap();
        }
    }

    pub fn set_height(&mut self, wasm_id: u32, dt: u32, v: f32) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get(&id) {
            let mut s = self.taffy.style(node.taffy_node).unwrap().clone();
            let height = match dt {
                1 => taffy::style::Dimension::length(v),
                2 => taffy::style::Dimension::percent(v / 100.0),
                _ => taffy::style::Dimension::auto(),
            };
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
                _ => taffy::prelude::FlexDirection::Row,
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
                _ => taffy::prelude::JustifyContent::FlexStart,
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
                _ => taffy::prelude::AlignItems::FlexStart,
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
                _ => taffy::prelude::FlexWrap::NoWrap,
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
                _ => taffy::prelude::AlignContent::FlexStart,
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
        if let Some(node) = self.nodes.get_mut(&id) {
            node.z_index = z;
        }
    }

    pub fn set_border_radius(&mut self, wasm_id: u32, r: f32) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.border_radius = r;
        }
    }

    pub fn set_opacity(&mut self, wasm_id: u32, opacity: f32) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.opacity = opacity.clamp(0.0, 1.0);
        }
    }

    pub fn set_clip_to_bounds(&mut self, wasm_id: u32, clip: bool) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.clip_to_bounds = clip;
        }
    }

    pub fn set_shadow(
        &mut self,
        wasm_id: u32,
        offset_x: f32,
        offset_y: f32,
        blur: f32,
        color: u32,
    ) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.shadow_offset_x = offset_x;
            node.shadow_offset_y = offset_y;
            node.shadow_blur = blur;
            node.shadow_color = color;
        }
    }

    pub fn set_blur(&mut self, wasm_id: u32, radius: f32) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.blur_radius = radius;
        }
    }

    pub fn set_position(&mut self, wasm_id: u32, x: f32, y: f32) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.position_x = x;
            node.position_y = y;
        }
    }

    pub fn set_border_width(&mut self, wasm_id: u32, width: f32) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.border_width = width;
        }
    }

    pub fn set_border_color(&mut self, wasm_id: u32, r: u8, g: u8, b: u8, a: u8) {
        let id = self.resolve_id(wasm_id);
        if let Some(node) = self.nodes.get_mut(&id) {
            node.border_color =
                ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        }
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
        if let Some(node) = self.nodes.get_mut(&id) {
            node.has_click = true;
        }
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
            if parent_id == 0 {
                break;
            }
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
            self.taffy
                .layout(node.taffy_node)
                .ok()
                .map(|l| (l.location.x, l.location.y, l.size.width, l.size.height))
        })
    }

    pub fn set_font_data(&mut self, data: Vec<u8>) {
        self.font_data = Some(data);
    }

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
        self.nodes
            .get(&id)
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
        self.nodes.insert(
            slot,
            ViewNode {
                taffy_node,
                color: Color::TRANSPARENT,
                children: vec![],
                parent_id: 0,
                z_index: 0,
                label: String::new(),
                text: String::new(),
                font_size: 16.0,
                font_family: String::new(),
                font_weight: 400,
                text_align: TextAlign::Start,
                border_radius: 0.0,
                opacity: 1.0,
                clip_to_bounds: false,
                shadow_offset_x: 0.0,
                shadow_offset_y: 0.0,
                shadow_blur: 0.0,
                shadow_color: 0xFF000000,
                blur_radius: 0.0,
                border_width: 0.0,
                border_color: 0xFF000000, // Black default
                position_x: 0.0,
                position_y: 0.0,
                role: Role::None,
                view_type: ViewType::Container,
                has_click: false,
                padding: (0.0, 0.0, 0.0, 0.0),
                dirty_fields: 0,
                last_measured_size: (0.0, 0.0),
            },
        );

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

    /// 扩容策略：预扩容（在达到80%容量时提前扩容，避免卡顿）
    pub fn should_pre_expand(&self) -> bool {
        let usage_ratio = self.nodes.len() as f32 / self.capacity as f32;
        usage_ratio > 0.8 && self.capacity < MAX_CAPACITY
    }

    /// 完整扩容逻辑（带预扩容检查）
    pub fn expand_capacity(&mut self, new_capacity: usize) -> Result<(), NodeError> {
        if new_capacity <= self.capacity {
            return Err(NodeError::InvalidCapacity);
        }
        if new_capacity > MAX_CAPACITY {
            return Err(NodeError::MaxCapacityExceeded);
        }

        // 找到最接近的容量档位
        let target_capacity = crate::CAPACITY_LEVELS
            .iter()
            .find(|&&level| level >= new_capacity)
            .copied()
            .unwrap_or(MAX_CAPACITY);

        self.capacity = target_capacity;
        log::info!(
            "Node capacity expanded to {}/{} (active: {})",
            target_capacity,
            MAX_CAPACITY,
            self.nodes.len()
        );

        Ok(())
    }

    /// 自动扩容（如果需要）
    pub fn auto_expand(&mut self) -> bool {
        if self.should_pre_expand() {
            if let Some(&next_level) = crate::CAPACITY_LEVELS
                .iter()
                .find(|&&level| level > self.capacity)
            {
                return self.expand_capacity(next_level).is_ok();
            }
        }
        false
    }

    /// 尝试扩容（简化版，用于 create_node_with_handle）
    fn try_expand_capacity(&mut self) -> bool {
        self.auto_expand()
    }

    /// 获取当前容量
    pub fn get_capacity(&self) -> usize {
        self.capacity
    }

    /// 获取代际数组（用于同步到 SharedBuffer）
    pub fn get_generations(&self) -> &[u32; MAX_CAPACITY] {
        &self.generations
    }

    // === Phase 3: 延迟回收与 LRU 淘汰 ===

    /// 延迟回收队列（节点进入回收状态，延迟几帧后正式回收）
    /// 防止异步操作访问已删除节点
    pub fn update_recycling(&mut self) {
        // 当前实现是立即回收，如需延迟回收可在此扩展
        // 例如：维护一个 countdown 队列，每帧减1，到0时正式回收
    }

    /// LRU 淘汰：当达到最大容量且需要新节点时，淘汰最久未使用的节点
    /// 返回被回收的节点 slot（供调用者处理状态保存）
    pub fn lru_recycle(&mut self) -> Option<u32> {
        if self.free_ids.is_empty() && self.nodes.len() >= MAX_CAPACITY {
            // 找到最久未使用的节点（简化：取最小编号）
            // 实际应维护访问时间戳
            let victim = self.nodes.keys().copied().min()?;

            // 增加代际并回收
            self.generations[victim as usize] = self.generations[victim as usize].wrapping_add(1);
            self.nodes.remove(&victim);

            // 清理映射
            self.active_handles.retain(|_, h| h.slot != victim);

            log::debug!("LRU recycled node slot {}", victim);
            Some(victim)
        } else {
            None
        }
    }

    /// 设置 SharedBuffer 指针
    pub fn set_shared_buffer_ptr(&mut self, ptr: *mut crate::SharedBuffer) {
        self.shared_buffer_ptr = if ptr.is_null() { None } else { Some(ptr) };
    }

    /// 获取 SharedBuffer 指针
    pub fn get_shared_buffer_ptr(&self) -> Option<*mut crate::SharedBuffer> {
        self.shared_buffer_ptr
    }

    /// 同步布局结果和代际到 SharedBuffer（供 Render 线程调用）
    /// 使用内部存储的 shared_buffer_ptr
    pub fn sync_to_shared_buffer(&self) {
        if let Some(ptr) = self.shared_buffer_ptr {
            unsafe {
                self.sync_to_shared_buffer_raw(ptr);
            }
        }
    }

    /// 同步布局结果和代际到 SharedBuffer（原始指针版本）
    ///
    /// # Safety
    /// 调用者必须确保 shared_buffer_ptr 有效
    pub unsafe fn sync_to_shared_buffer_raw(&self, shared_buffer_ptr: *mut crate::SharedBuffer) {
        if shared_buffer_ptr.is_null() {
            return;
        }

        let buffer = &mut *shared_buffer_ptr;

        // 同步容量
        buffer.capacity = self.capacity as u32;
        buffer.max_node_id = self.next_host_id;

        // 同步代际数组
        buffer.generations.copy_from_slice(&self.generations);

        // Build parent -> children mapping for topological traversal
        let mut children_map: std::collections::HashMap<u32, Vec<u32>> =
            std::collections::HashMap::new();
        let mut root_nodes = Vec::new();
        for (&id, node) in &self.nodes {
            if node.parent_id == 0 || node.parent_id == id {
                root_nodes.push(id);
            } else {
                children_map.entry(node.parent_id).or_default().push(id);
            }
        }

        // BFS from root to calculate absolute positions
        let mut queue = std::collections::VecDeque::from(root_nodes);
        let mut abs_positions: std::collections::HashMap<u32, (f32, f32)> =
            std::collections::HashMap::new();

        while let Some(id) = queue.pop_front() {
            if let Some(node) = self.nodes.get(&id) {
                // Get parent absolute position
                let (parent_abs_x, parent_abs_y) = abs_positions
                    .get(&node.parent_id)
                    .copied()
                    .unwrap_or((0.0, 0.0));

                // Get layout from Taffy
                if let Ok(layout) = self.taffy.layout(node.taffy_node) {
                    // Calculate absolute position
                    let abs_x = parent_abs_x + layout.location.x;
                    let abs_y = parent_abs_y + layout.location.y;
                    abs_positions.insert(id, (abs_x, abs_y));

                    // Write to shared buffer (using absolute position)
                    let slot_idx = id as usize;
                    if slot_idx < MAX_CAPACITY {
                        buffer.layout_results[slot_idx] = crate::LayoutResult {
                            x: abs_x,
                            y: abs_y,
                            width: layout.size.width,
                            height: layout.size.height,
                        };
                    }

                    // Process children
                    if let Some(children) = children_map.get(&id) {
                        for &child in children {
                            queue.push_back(child);
                        }
                    }
                }
            }
        }
    }

    /// 获取节点统计信息
    pub fn get_stats(&self) -> NodeStats {
        NodeStats {
            capacity: self.capacity,
            active_count: self.nodes.len(),
            free_count: self.free_ids.len(),
            total_created: self.next_host_id as u64,
            total_recycled: self.generations.iter().filter(|&&g| g > 0).count() as u64,
            expansion_count: self.get_expansion_count(),
        }
    }

    fn get_expansion_count(&self) -> u32 {
        crate::CAPACITY_LEVELS
            .iter()
            .position(|&level| level >= self.capacity)
            .map(|pos| pos as u32)
            .unwrap_or(0)
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod state_tests;

/// 节点错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeError {
    CapacityExceeded,
    MaxCapacityExceeded,
    InvalidCapacity,
    StaleHandle,
    NodeNotFound,
}

/// 节点统计信息
#[derive(Debug, Clone, Copy)]
pub struct NodeStats {
    pub capacity: usize,
    pub active_count: usize,
    pub free_count: usize,
    pub total_created: u64,
    pub total_recycled: u64,
    pub expansion_count: u32,
}

// === 调试和验证 ===

impl SharedState {
    /// 打印当前节点状态（用于调试）
    pub fn dump_state(&self) -> String {
        let stats = self.get_stats();
        let usage_pct = (stats.active_count as f32 / stats.capacity as f32) * 100.0;

        format!(
            "=== Node State ===\n\
             Capacity: {}/{} ({} expansions)\n\
             Active: {} ({:.1}%)\n\
             Free (recycled): {}\n\
             Total created: {}\n\
             Total recycled: {}\n\
             ==================",
            stats.capacity,
            MAX_CAPACITY,
            stats.expansion_count,
            stats.active_count,
            usage_pct,
            stats.free_count,
            stats.total_created,
            stats.total_recycled
        )
    }

    /// 验证代际ID系统完整性（用于测试）
    pub fn verify_generational_integrity(&self) -> Result<(), String> {
        for (slot, _node) in &self.nodes {
            let slot_idx = *slot as usize;
            if slot_idx >= MAX_CAPACITY {
                return Err(format!("Slot {} out of bounds", slot));
            }

            // 验证节点存在时，代际应该是正确的
            let _expected_gen = self.generations[slot_idx];
            // 注意：这里我们只是验证数据结构一致性
            // 实际的代际验证在 verify_handle 中
        }

        // 验证 free_ids 中的所有 ID 都对应非活跃节点
        for &slot in &self.free_ids {
            if self.nodes.contains_key(&slot) {
                return Err(format!("Free ID {} still has active node", slot));
            }
        }

        Ok(())
    }
}
