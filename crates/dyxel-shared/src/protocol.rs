// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

macro_rules! define_protocol {
    (
        $dol:tt,
        $( [$id:expr] $name:ident ($($arg:ident : $typ:ty),*) ),* $(,)?
    ) => {
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum OpCode {
            $( $name = $id, )*
        }

        // Generate stricter hash including parameter types
        pub const PROTOCOL_HASH: u64 = $crate::fnv1a_hash(concat!(
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

// Single source of truth: define opcodes and their parameter types
define_protocol! {
    $,
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
