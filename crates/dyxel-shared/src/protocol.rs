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
    // === Core Node Operations (1-31) ===
    [1] CreateNode(id: u32),
    [2] SetViewType(id: u32, vt: u32),
    [3] SetColor(id: u32, r: u8, g: u8, b: u8),
    [4] SetWidth(id: u32, dt: u8, v: f32),
    [5] SetHeight(id: u32, dt: u8, v: f32),
    [6] SetFlexDirection(id: u32, dir: u32),
    [7] SetJustifyContent(id: u32, j: u32),
    [8] SetAlignItems(id: u32, a: u32),
    [9] SetFlexGrow(id: u32, grow: f32),
    [10] SetZIndex(id: u32, z: i32),
    [11] SetFontSize(id: u32, size: f32),
    [12] SetBorderRadius(id: u32, r: f32),
    [13] SetPadding(id: u32, t: f32, r: f32, b: f32, l: f32),
    [14] SetFlexWrap(id: u32, wrap: u32),
    [15] SetAlignContent(id: u32, ac: u32),
    
    // === Interaction & Events (16-31) ===
    [16] AttachClick(id: u32),
    [17] SetText(id: u32, len: u32), 
    [18] AddChild(pid: u32, cid: u32),
    [19] SetSemantics(id: u32, role: u32),
    [20] SetLabel(id: u32, len: u32),
    [21] UpdateLayout(),
    [22] SelectNode(id: u32),
    
    // === Compact Operations (23-25) ===
    [23] SetColorCompact(r: u8, g: u8, b: u8, a: u8),
    [24] SetWidthCompact(dt: u8, v: f32),
    [25] SetHeightCompact(dt: u8, v: f32),
    
    // === Gesture Handler Registration (28-31) ===
    // WASM notifies Host which nodes have gesture handlers
    // Tap handler with configurable count (1=single, 2=double, 3=triple, etc.)
    [28] RegisterTapHandler(id: u32, count: u32),
    [29] RegisterLongPressHandler(id: u32),
    [30] RegisterPanHandler(id: u32),
    // [31] Reserved - was RegisterDoubleTapHandler, now merged into RegisterTapHandler
    // Note: UnregisterGestureHandler moved to 90 to avoid conflict
    
    // === Rich Text Operations (32-47) ===
    [32] CreateTextNode(id: u32),
    [33] CreateSpanNode(id: u32),
    [34] CreateRichTextNode(id: u32),
    [35] SetTextContent(id: u32, len: u32),
    [36] SetTextColor(id: u32, r: u8, g: u8, b: u8, a: u8),
    [37] SetTextWeight(id: u32, weight: u16),
    [38] SetTextFontFamily(id: u32, len: u32),

    // === Gesture Handler Registration Extended (39-40) ===
    [39] RegisterScaleHandler(id: u32),
    [40] RegisterRotationHandler(id: u32),
    // Note: RegisterMultiTapHandler removed - use RegisterTapHandler with count instead

    // === Unified Gesture Registration (26-27) - Phase 1 ===
    // Replaces [28,29,30,39,40,90] with unified mask-based registration
    [26] RegisterGesture(id: u32, mask: u16),  // bitflags: Tap|LongPress|Pan|Scale|Rotation
    [27] SetGestureConfig(id: u32, config_type: u8, value: u32),  // config_type: 0=tap_count, 1=timeout, etc.

    // === Transaction Operations (48-51) ===
    [48] BeginTransaction(seq_id: u32, flags: u16),
    [49] EndTransaction(seq_id: u32),
    [50] AbortTransaction(seq_id: u32),
    [51] SetNodeDirty(id: u32, fields: u8),
    
    // === LayoutRegistry Operations (52-55) - NEW! ===
    [52] GetLayout(id: u32),
    [53] IsLayoutDirty(id: u32),
    [54] ClearLayoutDirty(id: u32),
    [55] GetLayoutBatch(start_id: u32, count: u32),
    
    // === Gesture Events (56-63) - Legacy, bubble in WASM ===
    [56] GestureTap(node_id: u32, x: f32, y: f32),
    [57] GestureDoubleTap(node_id: u32, x: f32, y: f32),
    [58] GestureLongPressStart(node_id: u32, x: f32, y: f32),
    [59] GestureLongPressEnd(node_id: u32, x: f32, y: f32),
    [60] GesturePanStart(node_id: u32, x: f32, y: f32),
    [61] GesturePanUpdate(node_id: u32, x: f32, y: f32, delta_x: f32, delta_y: f32),
    [62] GesturePanEnd(node_id: u32, x: f32, y: f32, velocity_x: f32, velocity_y: f32),
    [63] GestureCancel(node_id: u32),
    
    // === Direct Gesture Events (72-79) - Host resolves bubbling ===
    // These events have already been resolved by Host using HandlerRegistry
    // WASM should call the handler directly without bubbling
    [72] DirectGestureTap(node_id: u32, x: f32, y: f32),
    [73] DirectGestureDoubleTap(node_id: u32, x: f32, y: f32),
    [74] DirectGestureLongPress(node_id: u32, x: f32, y: f32),
    [75] DirectGesturePanStart(node_id: u32, x: f32, y: f32),
    [76] DirectGesturePanUpdate(node_id: u32, x: f32, y: f32, delta_x: f32, delta_y: f32),
    [77] DirectGesturePanEnd(node_id: u32, x: f32, y: f32),
    [78] DirectGestureScaleStart(node_id: u32, x: f32, y: f32, scale: f32),
    [79] DirectGestureScaleUpdate(node_id: u32, x: f32, y: f32, scale: f32, delta_scale: f32),
    [80] DirectGestureScaleEnd(node_id: u32, x: f32, y: f32),
    [81] DirectGestureRotationStart(node_id: u32, x: f32, y: f32, angle: f32),
    [82] DirectGestureRotationUpdate(node_id: u32, x: f32, y: f32, angle: f32, delta_angle: f32),
    [83] DirectGestureRotationEnd(node_id: u32, x: f32, y: f32),
    [84] DirectGestureLongPressEnd(node_id: u32, x: f32, y: f32),

    // === Device Info (64) ===
    [64] UpdateDeviceInfo(dpr: f32, text_scale: f32, width: f32, height: f32, safe_top: f32, safe_bottom: f32, platform: u32),

    // === Gesture Handler Unregistration (90) ===
    [90] UnregisterGestureHandler(id: u32), // Generic unregister

    // === Unified Gesture Events (85-89) - Phase 2 ===
    // Replaces [56-63] and [72-84] with 5 unified events
    // event_type: 0=Tap, 1=LongPress, 2=Pan, 3=Scale, 4=Rotation
    // phase: 0=Began, 1=Changed, 2=Ended, 3=Cancelled (discrete events use 2=Ended)
    [85] GestureEventV2(node_id: u32, event_type: u8, phase: u8, x: f32, y: f32),
    // Extended event with payload (tap_count, scale, delta_x, etc. encoded in payload)
    [86] GestureEventV2Ex(node_id: u32, event_type: u8, phase: u8, x: f32, y: f32, payload: u32),
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutResult {
    pub x: f32, pub y: f32, pub width: f32, pub height: f32,
}

pub const MAX_COMMAND_BYTES: usize = 1024 * 64;
pub const MAX_NODES: usize = 1024;

/// 容量档位（支持动态扩容）
pub const CAPACITY_LEVELS: [usize; 5] = [256, 512, 1024, 2048, 4096];
pub const MAX_CAPACITY: usize = 4096;
pub const INITIAL_CAPACITY: usize = 256;

/// 带代际的节点句柄（防止 Stale ID）
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeHandle {
    /// 槽位索引
    pub slot: u32,
    /// 代际计数器（每次回收+1）
    pub generation: u32,
}

impl NodeHandle {
    /// 无效句柄
    pub const INVALID: Self = Self { slot: u32::MAX, generation: 0 };
    
    /// 创建新句柄
    pub const fn new(slot: u32, generation: u32) -> Self {
        Self { slot, generation }
    }
    
    /// 检查是否有效
    pub fn is_valid(&self) -> bool {
        self.slot != u32::MAX
    }
}

impl Default for NodeHandle {
    fn default() -> Self {
        Self::INVALID
    }
}

/// Transaction flags for controlling behavior
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionFlags {
    None = 0,
    /// Skip rendering if only layout properties changed
    SkipIfLayoutOnly = 1 << 0,
    /// Force immediate application (no batching)
    Immediate = 1 << 1,
    /// Allow merging with previous transaction
    Mergeable = 1 << 2,
}

impl TransactionFlags {
    pub fn from_bits(bits: u16) -> Self {
        match bits {
            0 => Self::None,
            1 => Self::SkipIfLayoutOnly,
            2 => Self::Immediate,
            4 => Self::Mergeable,
            _ => Self::None,
        }
    }
}

/// Node dirty field bitflags for tracking what changed
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyField {
    None = 0,
    Position = 1 << 0,  // x, y changed
    Size = 1 << 1,      // width, height changed
    Style = 1 << 2,     // color, border, etc.
    Text = 1 << 3,      // text content changed
    Children = 1 << 4,  // child add/remove
    Layout = 1 << 5,    // flex properties changed
}

impl DirtyField {
    pub fn from_bits(bits: u8) -> Self {
        Self::from_bits_truncate(bits)
    }
    
    pub fn from_bits_truncate(bits: u8) -> Self {
        match bits {
            0 => Self::None,
            1 => Self::Position,
            2 => Self::Size,
            4 => Self::Style,
            8 => Self::Text,
            16 => Self::Children,
            32 => Self::Layout,
            _ => Self::None,
        }
    }
    
    pub fn bits(&self) -> u8 {
        *self as u8
    }
    
    pub fn contains(&self, other: Self) -> bool {
        self.bits() & other.bits() != 0
    }
}

/// Dirty region tracker using bitset
#[derive(Debug, Default, Clone)]
pub struct DirtyTracker {
    /// Bitset: 1 bit per node, 1024 nodes = 32 u32 words
    pub node_bitset: [u32; 32],
    /// Track which fields changed per node (for selective re-render)
    /// Store as u8 to allow bit combinations (e.g., Style | Size)
    pub node_dirty_fields: std::collections::HashMap<u32, u8>,
    /// Global dirty flag - any change occurred
    pub any_dirty: bool,
}

impl DirtyTracker {
    pub fn new() -> Self {
        Self {
            node_bitset: [0; 32],
            node_dirty_fields: std::collections::HashMap::new(),
            any_dirty: false,
        }
    }
    
    /// Mark a node as dirty
    pub fn mark_dirty(&mut self, node_id: u32, fields: DirtyField) {
        if node_id as usize >= 1024 { return; }
        
        let word_idx = (node_id / 32) as usize;
        let bit_idx = node_id % 32;
        self.node_bitset[word_idx] |= 1 << bit_idx;
        
        // Accumulate dirty fields as raw bits to preserve combinations
        let field_bits = fields.bits();
        self.node_dirty_fields
            .entry(node_id)
            .and_modify(|f| *f |= field_bits)
            .or_insert(field_bits);
        
        self.any_dirty = true;
    }
    
    /// Check if a node is dirty
    pub fn is_node_dirty(&self, node_id: u32) -> bool {
        if node_id as usize >= 1024 { return false; }
        let word_idx = (node_id / 32) as usize;
        let bit_idx = node_id % 32;
        (self.node_bitset[word_idx] >> bit_idx) & 1 != 0
    }
    
    /// Check if any nodes are dirty
    pub fn has_dirty(&self) -> bool {
        self.any_dirty
    }
    
    /// Clear all dirty flags
    pub fn clear(&mut self) {
        self.node_bitset = [0; 32];
        self.node_dirty_fields.clear();
        self.any_dirty = false;
    }
    
    /// Iterate over all dirty node IDs
    pub fn iter_dirty_nodes(&self) -> impl Iterator<Item = u32> + '_ {
        self.node_bitset.iter().enumerate().flat_map(|(word_idx, &word)| {
            let mut nodes = Vec::new();
            let mut w = word;
            while w != 0 {
                let bit = w.trailing_zeros();
                nodes.push((word_idx as u32 * 32) + bit);
                w &= w - 1;  // Clear lowest set bit
            }
            nodes.into_iter()
        })
    }
}

#[repr(C, align(16))]
pub struct SharedBuffer {
    pub command_len: u32,
    pub max_node_id: u32,
    /// 当前容量（支持动态扩容，最大 MAX_CAPACITY）
    pub capacity: u32,
    pub _padding: [u32; 1],
    pub command_data: [u8; MAX_COMMAND_BYTES],
    /// 布局结果（实际使用 capacity，预分配 MAX_CAPACITY）
    pub layout_results: [LayoutResult; MAX_CAPACITY],
    /// 代际数组（与 layout_results 一一对应）
    pub generations: [u32; MAX_CAPACITY],
    /// 脏标记（位图，大小取决于 capacity）
    pub dirty_mask: [u32; 128],  // 4096 / 32 = 128
    /// Input event ring buffer (for Input Proxy)
    pub input_buffer: crate::input::InputBuffer,
    /// Device information (read by WASM)
    pub device_info: crate::device::DeviceInfo,
}
