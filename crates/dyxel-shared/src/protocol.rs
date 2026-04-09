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

    // === Gesture Handler Registration V2 (41-45) - Custom Config Support ===
    // V2 handlers support custom slop, timeout, and direction parameters
    [41] RegisterTapHandlerV2(id: u32, count: u8, multi_click_gap_ms: u16),
    [42] RegisterLongPressHandlerV2(id: u32, timeout_ms: u16, slop: u8),
    [43] RegisterPanHandlerV2(id: u32, slop: u8, direction: u8), // direction: 0=any, 1=horizontal, 2=vertical
    [44] RegisterScaleHandlerV2(id: u32, slop: u8),
    [45] RegisterRotationHandlerV2(id: u32, slop: u8),

    // === Text Styling (46) ===
    [46] SetTextAlign(id: u32, align: u8), // 0=Start, 1=Center, 2=End, 3=Justified

    // === TextInput Operations (100-114) ===
    [100] CreateTextInput(id: u32),
    [101] SetTextInputFocused(id: u32, focused: u8),
    [102] SyncTextState(id: u32, len: u32, sel_start: u32, sel_end: u32),
    [103] SetTextInputCursor(id: u32, pos: u32),
    [104] SetTextInputSelection(id: u32, start: u32, end: u32),
    [105] SetTextInputType(id: u32, input_type: u8),
    [106] ShowTextInputKeyboard(),
    [107] HideTextInputKeyboard(),
    [108] CopyToClipboard(id: u32),
    [109] CutToClipboard(id: u32),
    [110] RequestPasteFromClipboard(id: u32),
    // === Context Menu (115-119) ===
    [115] ShowTextInputContextMenu(id: u32),
    [116] HideTextInputContextMenu(id: u32),
    [117] TextInputMenuItemSelected(id: u32, item: u8), // 0=select_all, 1=copy, 2=paste, 3=cut
    [118] SetComposingText(id: u32, len: u32), // IME composition in progress
    [119] CommitComposingText(id: u32), // IME composition finished
    [111] SetTextInputPlaceholder(id: u32, len: u32),
    [112] SetTextInputMaxLength(id: u32, max_len: u32),
    [113] SetTextInputReturnKeyType(id: u32, key_type: u8),
    [114] SetTextInputSecure(id: u32, secure: u8), // password mode

    // === TextInput Events (Host -> Guest) (120-129) ===
    [120] OnTextUpdate(id: u32, len: u32, sel_start: u32, sel_end: u32),
    [121] TextInputSelectionChanged(id: u32, start: u32, end: u32),
    [122] KeyboardHeightChanged(height: f32, animation_duration_ms: u32),
    [123] PasteFromClipboard(id: u32, len: u32),
    [124] TextInputFocusChanged(id: u32, focused: u8),
    [125] ComposingRegionChanged(id: u32, start: u32, end: u32), // IME composition range
    [126] KeyboardWillShow(height: f32, animation_duration_ms: u32, animation_curve: u8),
    [127] KeyboardWillHide(animation_duration_ms: u32, animation_curve: u8),

    // === Unified Gesture Registration (26-27) - Phase 1 ===
    // Replaces [28,29,30,39,40,90] with unified mask-based registration
    [26] RegisterGesture(id: u32, mask: u16),  // bitflags: Tap|LongPress|Pan|Scale|Rotation
    [27] SetGestureConfig(id: u32, config_type: u8, value: u32),  // config_type: 0=tap_count, 1=timeout, etc.

    // === Transaction Operations (48-51) ===
    [48] BeginTransaction(seq_id: u32, flags: u16),
    [49] EndTransaction(seq_id: u32),
    [50] AbortTransaction(seq_id: u32),
    [51] SetNodeDirty(id: u32, fields: u8),

    // === LayoutRegistry Operations (52-55) ===
    [52] GetLayout(id: u32),
    [53] IsLayoutDirty(id: u32),
    [54] ClearLayoutDirty(id: u32),
    [55] GetLayoutBatch(start_id: u32, count: u32),

    // === Pointer Event Registration (56-57) - For press/hover effects ===
    [56] RegisterPointerDownHandler(id: u32),
    [57] RegisterPointerUpHandler(id: u32),

    // === Device Info (64) ===
    [64] UpdateDeviceInfo(dpr: f32, text_scale: f32, width: f32, height: f32, safe_top: f32, safe_bottom: f32, platform: u32),

    // === Gesture Handler Unregistration (90) ===
    [90] UnregisterGestureHandler(id: u32), // Generic unregister

    // === Unified Gesture Events (85-86) ===
    // Host -> WASM gesture events
    // event_type: 0=Tap, 1=LongPress, 2=Pan, 3=Scale, 4=Rotation
    // phase: 0=Began, 1=Changed, 2=Ended, 3=Cancelled (discrete events use 2=Ended)
    [85] GestureEventV2(node_id: u32, event_type: u8, phase: u8, x: f32, y: f32),
    // Extended event with payload (tap_count, scale, delta_x, etc. encoded in payload)
    [86] GestureEventV2Ex(node_id: u32, event_type: u8, phase: u8, x: f32, y: f32, payload: u32),

    // === Layer Effects (92-96) - Vello Native Layer Rendering ===
    [92] SetOpacity(id: u32, opacity: f32),           // 0.0 - 1.0
    [93] SetShadow(id: u32, offset_x: f32, offset_y: f32, blur: f32, color: u32), // color: RGBA
    [94] SetBlur(id: u32, radius: f32),               // Gaussian blur radius
    [95] SetClipToBounds(id: u32, clip: u8),          // 0=false, 1=true
    [96] SetPosition(id: u32, x: f32, y: f32),        // Absolute position offset

    // === Border / Stroke (97-98) ===
    [97] SetBorderWidth(id: u32, width: f32),         // Border stroke width
    [98] SetBorderColor(id: u32, r: u8, g: u8, b: u8, a: u8), // Border color RGBA
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutResult {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
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
    pub const INVALID: Self = Self {
        slot: u32::MAX,
        generation: 0,
    };

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
    Position = 1 << 0, // x, y changed
    Size = 1 << 1,     // width, height changed
    Style = 1 << 2,    // color, border, etc.
    Text = 1 << 3,     // text content changed
    Children = 1 << 4, // child add/remove
    Layout = 1 << 5,   // flex properties changed
}

// =============================================================================
// Gesture Event Buffer (Host → WASM)
// =============================================================================

/// Gesture event types for Host → WASM communication
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureEventType {
    Tap = 0,
    LongPress = 1,
    Pan = 2,
    Scale = 3,
    Rotation = 4,
}

impl GestureEventType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Tap),
            1 => Some(Self::LongPress),
            2 => Some(Self::Pan),
            3 => Some(Self::Scale),
            4 => Some(Self::Rotation),
            _ => None,
        }
    }
}

/// Gesture phase for continuous gestures
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GesturePhase {
    Began = 0,
    Changed = 1,
    Ended = 2,
    Cancelled = 3,
}

impl GesturePhase {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Began),
            1 => Some(Self::Changed),
            2 => Some(Self::Ended),
            3 => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// Host → WASM gesture event
/// Fixed 32-byte size for predictable memory layout
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HostGestureEvent {
    /// Target node ID
    pub node_id: u32,
    /// Event type (GestureEventType)
    pub event_type: u8,
    /// Phase (GesturePhase)
    pub phase: u8,
    /// Padding for alignment
    pub _padding: [u8; 2],
    /// X position
    pub x: f32,
    /// Y position
    pub y: f32,
    /// Delta X (for Pan/Scale/Rotation updates)
    pub delta_x: f32,
    /// Delta Y (for Pan updates)
    pub delta_y: f32,
    /// Scale value (for Scale events)
    pub scale: f32,
    /// Rotation angle in radians (for Rotation events)
    pub rotation: f32,
    /// Tap count (for Tap events)
    pub tap_count: u32,
    /// Velocity X (for Pan end)
    pub velocity_x: f32,
    /// Velocity Y (for Pan end)
    pub velocity_y: f32,
}

impl Default for HostGestureEvent {
    fn default() -> Self {
        Self {
            node_id: 0,
            event_type: 0,
            phase: 0,
            _padding: [0; 2],
            x: 0.0,
            y: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            scale: 1.0,
            rotation: 0.0,
            tap_count: 1,
            velocity_x: 0.0,
            velocity_y: 0.0,
        }
    }
}

/// Gesture event buffer capacity
/// At 60 FPS with complex gestures, ~50 events/sec is plenty
pub const GESTURE_BUFFER_CAPACITY: usize = 64;

/// Host → WASM gesture event ring buffer
///
/// Single-producer single-consumer model:
/// - Producer: Host-side gesture recognizer
/// - Consumer: WASM logic thread (dyxel_view_tick)
#[repr(C)]
pub struct GestureEventBuffer {
    /// Write position (host-side monotonic increment)
    pub write_idx: u32,
    /// Read position (WASM-side monotonic increment)
    pub read_idx: u32,
    /// Overflow count (for debugging)
    pub overflow_count: u32,
    /// Reserved
    pub _reserved: u32,
    /// Event storage array
    pub events: [HostGestureEvent; GESTURE_BUFFER_CAPACITY],
}

impl GestureEventBuffer {
    /// Create empty buffer
    pub const fn new() -> Self {
        Self {
            write_idx: 0,
            read_idx: 0,
            overflow_count: 0,
            _reserved: 0,
            events: [HostGestureEvent {
                node_id: 0,
                event_type: 0,
                phase: 0,
                _padding: [0; 2],
                x: 0.0,
                y: 0.0,
                delta_x: 0.0,
                delta_y: 0.0,
                scale: 1.0,
                rotation: 0.0,
                tap_count: 1,
                velocity_x: 0.0,
                velocity_y: 0.0,
            }; GESTURE_BUFFER_CAPACITY],
        }
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.read_idx == self.write_idx
    }

    /// Check if buffer is full
    pub fn is_full(&self) -> bool {
        self.write_idx - self.read_idx >= GESTURE_BUFFER_CAPACITY as u32
    }

    /// Current event count
    pub fn len(&self) -> usize {
        (self.write_idx - self.read_idx) as usize
    }

    /// Push event (called by host)
    /// Returns true on success, false if buffer full (event dropped)
    pub fn push(&mut self, event: HostGestureEvent) -> bool {
        if self.is_full() {
            self.overflow_count += 1;
            return false;
        }
        let idx = (self.write_idx % GESTURE_BUFFER_CAPACITY as u32) as usize;
        self.events[idx] = event;
        self.write_idx += 1;
        true
    }

    /// Pop event (called by WASM)
    /// Returns None if buffer empty
    pub fn pop(&mut self) -> Option<HostGestureEvent> {
        if self.is_empty() {
            return None;
        }
        let idx = (self.read_idx % GESTURE_BUFFER_CAPACITY as u32) as usize;
        let event = self.events[idx];
        self.read_idx += 1;
        Some(event)
    }

    /// Batch read all available events
    pub fn drain(&mut self) -> GestureEventDrainIterator<'_> {
        GestureEventDrainIterator { buffer: self }
    }

    /// Clear buffer
    pub fn clear(&mut self) {
        self.read_idx = self.write_idx;
    }
}

/// Gesture event buffer batch read iterator
pub struct GestureEventDrainIterator<'a> {
    buffer: &'a mut GestureEventBuffer,
}

impl<'a> Iterator for GestureEventDrainIterator<'a> {
    type Item = HostGestureEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.buffer.pop()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.buffer.len();
        (len, Some(len))
    }
}

impl<'a> ExactSizeIterator for GestureEventDrainIterator<'a> {}

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
        if node_id as usize >= 1024 {
            return;
        }

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
        if node_id as usize >= 1024 {
            return false;
        }
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
        self.node_bitset
            .iter()
            .enumerate()
            .flat_map(|(word_idx, &word)| {
                let mut nodes = Vec::new();
                let mut w = word;
                while w != 0 {
                    let bit = w.trailing_zeros();
                    nodes.push((word_idx as u32 * 32) + bit);
                    w &= w - 1; // Clear lowest set bit
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
    pub dirty_mask: [u32; 128], // 4096 / 32 = 128
    /// Input event ring buffer (for Input Proxy)
    pub input_buffer: crate::input::InputBuffer,
    /// Device information (read by WASM)
    pub device_info: crate::device::DeviceInfo,
}
