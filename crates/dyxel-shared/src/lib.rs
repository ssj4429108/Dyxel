// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use serde::{Serialize, Deserialize};

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum FlexDirection { Row = 0, Column = 1, RowReverse = 2, ColumnReverse = 3 }
#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum JustifyContent { FlexStart = 0, Center = 1, FlexEnd = 2, SpaceBetween = 3, SpaceAround = 4, SpaceEvenly = 5 }
#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum AlignItems { FlexStart = 0, Center = 1, FlexEnd = 2, Stretch = 3 }
#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum PositionType { Relative = 0, Absolute = 1 }

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum Dimension { Auto, Pixels(f32), Percent(f32) }

impl From<&str> for Dimension {
    fn from(s: &str) -> Self {
        if s == "auto" { Dimension::Auto }
        else if s.ends_with('%') { Dimension::Percent(s[..s.len()-1].parse().unwrap_or(0.0)) }
        else { Dimension::Pixels(s.parse().unwrap_or(0.0)) }
    }
}
impl From<f32> for Dimension { fn from(f: f32) -> Self { Dimension::Pixels(f) } }
impl From<i32> for Dimension { fn from(i: i32) -> Self { Dimension::Pixels(i as f32) } }

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { None = 0, Button = 1, Heading = 2, Link = 3, Label = 4 }

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewType { Container = 0, Text = 1, Button = 2, Image = 3, Input = 4 }

// Define a simple compile-time hash algorithm for verifying protocol consistency
pub const fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        i += 1;
    }
    hash
}

macro_rules! define_protocol {
    (
        $( [$id:expr] $name:ident ($($arg:ident : $typ:ty),*) ),* $(,)?
    ) => {
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum OpCode {
            $( $name = $id, )*
        }

        // Generate stricter hash including parameter types
        pub const PROTOCOL_HASH: u64 = fnv1a_hash(concat!(
            $( stringify!($name), $( stringify!($typ), )* )*
        ).as_bytes());

        impl OpCode {
            pub const fn data_len(&self) -> usize {
                match self {
                    $(
                        OpCode::$name => {
                            let sum = 0;
                            $( let sum = sum + std::mem::size_of::<$typ>(); )*
                            sum
                        }
                    )*
                }
            }

            pub fn from_u8(v: u8) -> Option<Self> {
                match v {
                    $( $id => Some(OpCode::$name), )*
                    _ => None,
                }
            }
        }

        // Use nested macro trick to generate push_command! and dispatch_op! macros with $ symbols
        macro_rules! generate_helpers {
            ($dol:tt) => {
                #[macro_export]
                macro_rules! push_command {
                    ( $dol buffer:expr, $dol op:ident $dol(, $dol val:expr)* ) => {
                        #[allow(unused_assignments)]
                        unsafe {
                            let mut offset = $dol buffer.command_len as usize;
                            let mut data_size = 0;
                            $dol( data_size += std::mem::size_of_val(&$dol val); )*
                            
                            if offset + 1 + data_size <= $crate::MAX_COMMAND_BYTES {
                                $dol buffer.command_data[offset] = $crate::OpCode::$dol op as u8;
                                offset += 1;
                                $dol(
                                    let bytes = $dol val.to_le_bytes();
                                    let n = bytes.len();
                                    $dol buffer.command_data[offset..offset+n].copy_from_slice(&bytes);
                                    offset += n;
                                )*
                                $dol buffer.command_len = offset as u32;
                            }
                        }
                    };
                }

                #[macro_export]
                macro_rules! dispatch_op {
                    ($dol op:expr, $dol buf:expr, $dol offset:expr, $dol body:ident, $dol($dol state:tt)*) => {
                        match $dol op {
                            $(
                                $crate::OpCode::$name => {
                                    $(
                                        let $arg = <$typ>::from_le_bytes(
                                            $dol buf[$dol offset .. $dol offset + std::mem::size_of::<$typ>()]
                                                .try_into()
                                                .expect("Decoding failed")
                                        );
                                        $dol offset += std::mem::size_of::<$typ>();
                                    )*
                                    $dol body ! ($name, $dol($dol state)* $(, $arg)*);
                                }
                            )*
                        }
                    };
                }
            };
        }
        generate_helpers!($);
    };
}

// Single source of truth: define opcodes and their parameter types
define_protocol! {
    [1] CreateNode(id: u32),
    [2] SetViewType(id: u32, vt: u32),
    [3] SetColor(id: u32, r: u8, g: u8, b: u8),
    [4] SetWidth(id: u32, dt: u8, v: f32),
    [5] SetHeight(id: u32, dt: u8, v: f32),
    [6] SetFlexDirection(id: u32, dir: u32),
    [7] SetJustifyContent(id: u32, j: u32),
    [8] SetAlignItems(id: u32, a: u32),
    [9] SetPosition(id: u32, p: u32),
    [10] SetInset(id: u32, t: f32, r: f32, b: f32, l: f32),
    [11] SetFlexGrow(id: u32, grow: f32),
    [12] SetZIndex(id: u32, z: i32),
    [13] SetFontSize(id: u32, size: f32),
    [14] SetBorderRadius(id: u32, r: f32),
    [15] SetPadding(id: u32, t: f32, r: f32, b: f32, l: f32),
    [16] AttachClick(id: u32),
    [17] SetText(id: u32, len: u32), 
    [18] AddChild(pid: u32, cid: u32),
    [19] SetSemantics(id: u32, role: u32),
    [20] SetLabel(id: u32, len: u32),
    [21] UpdateLayout(),
    [22] SelectNode(id: u32),
    [23] SetColorCompact(r: u8, g: u8, b: u8, a: u8),
    [24] SetWidthCompact(dt: u8, v: f32),
    [25] SetHeightCompact(dt: u8, v: f32),
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutResult {
    pub x: f32, pub y: f32, pub width: f32, pub height: f32,
}

pub const MAX_COMMAND_BYTES: usize = 1024 * 64;
pub const MAX_NODES: usize = 1024;

#[repr(C, align(16))]
pub struct SharedBuffer {
    pub command_len: u32,
    pub max_node_id: u32,
    pub _padding: [u32; 2],
    pub command_data: [u8; MAX_COMMAND_BYTES],
    pub layout_results: [LayoutResult; MAX_NODES],
    pub dirty_mask: [u32; 32], 
}

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
