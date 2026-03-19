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

// 定义一个简单的编译期哈希算法，用于验证协议一致性
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

        // 生成包含参数类型的更严格哈希
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

        // 使用嵌套宏技巧来生成带 $ 符号的 push_command! 和 dispatch_op! 宏
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

// 单一数据源：定义操作码及其参数类型
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
