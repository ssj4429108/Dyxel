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

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    CreateNode = 1, SetViewType = 2, SetColor = 3, SetWidth = 4, SetHeight = 5,
    SetFlexDirection = 6, SetJustifyContent = 7, SetAlignItems = 8, SetPosition = 9,
    SetInset = 10, SetFlexGrow = 11, SetZIndex = 12, SetFontSize = 13, SetBorderRadius = 14,
    SetPadding = 15, AttachClick = 16, SetText = 17, AddChild = 18, SetSemantics = 19,
    SetLabel = 20, UpdateLayout = 21,
    SelectNode = 22, // 显式选择后续指令的作用节点
    SetColorCompact = 23, // 作用于当前选中节点 (4字节: RGBA)
    SetWidthCompact = 24, // 作用于当前选中节点 (5字节: Type + Val)
    SetHeightCompact = 25, // 作用于当前选中节点 (5字节: Type + Val)
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
    pub dirty_mask: [u32; 32], // 1024 bits for 1024 nodes
}
