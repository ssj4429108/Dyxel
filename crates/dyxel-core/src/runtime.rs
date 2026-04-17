// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::handler_registry::{HandlerRegistry, HandlerType};
use crate::state::SharedState;
use crate::transaction::{
    get_dirty_field_for_opcode, DirtyTracker, StagedCommand, TransactionProcessor, TransactionState,
};
use dyxel_shared::{DirtyField, OpCode, MAX_COMMAND_BYTES, MAX_NODES};
use std::sync::{Arc, Mutex, OnceLock};

/// Global transaction processor for non-WASM targets
#[cfg(not(target_arch = "wasm32"))]
static TX_PROCESSOR: OnceLock<Mutex<TransactionProcessor>> = OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
fn get_tx_processor() -> &'static Mutex<TransactionProcessor> {
    TX_PROCESSOR.get_or_init(|| Mutex::new(TransactionProcessor::new()))
}

/// Global handler registry for gesture handlers
#[cfg(not(target_arch = "wasm32"))]
static HANDLER_REGISTRY: OnceLock<Mutex<HandlerRegistry>> = OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
pub fn get_handler_registry() -> &'static Mutex<HandlerRegistry> {
    HANDLER_REGISTRY.get_or_init(|| Mutex::new(HandlerRegistry::new()))
}

/// Check if render is needed based on dirty tracker
#[cfg(not(target_arch = "wasm32"))]
pub fn is_render_needed() -> bool {
    get_tx_processor().lock().unwrap().take_render_pending()
}

/// Get the dirty tracker for render optimization
#[cfg(not(target_arch = "wasm32"))]
pub fn get_dirty_tracker() -> Option<DirtyTracker> {
    // Return a clone since we can't hold the lock across the return
    Some(get_tx_processor().lock().unwrap().dirty_tracker.clone())
}

/// Clear dirty tracker after render
#[cfg(not(target_arch = "wasm32"))]
pub fn clear_dirty_tracker() {
    get_tx_processor().lock().unwrap().dirty_tracker.clear();
}

/// Mark all nodes as dirty after layout computation
/// Called by Render thread after compute_layout to ensure Logic thread syncs layout to WASM
#[cfg(not(target_arch = "wasm32"))]
pub fn mark_all_nodes_dirty(node_ids: &[u32]) {
    let mut tx = get_tx_processor().lock().unwrap();
    for &id in node_ids {
        tx.dirty_tracker.mark_dirty(id, DirtyField::Layout);
    }
}

/// Command context for processing
struct CommandContext {
    cur_id: Option<u32>,
}

impl CommandContext {
    fn new() -> Self {
        Self { cur_id: None }
    }
}

#[allow(unused_macros)]
macro_rules! handle_op {
    // Extract node_id from various patterns and stage command
    (CreateNode, $ctx:ident, $offset:ident, $payload:ident, $id:expr) => {{
        $ctx.cur_id = Some($id);
        Some(($id, DirtyField::None))
    }};
    (CreateTextNode, $ctx:ident, $offset:ident, $payload:ident, $id:expr) => {{
        $ctx.cur_id = Some($id);
        Some(($id, DirtyField::None))
    }};
    (SelectNode, $ctx:ident, $offset:ident, $payload:ident, $id:expr) => {{
        $ctx.cur_id = Some($id);
        None // No dirty tracking for SelectNode
    }};
    (SetColor, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $r:expr, $g:expr, $b:expr) => {{
        Some(($id, DirtyField::Style))
    }};
    (SetColorCompact, $ctx:ident, $offset:ident, $payload:ident, $r:expr, $g:expr, $b:expr, $a:expr) => {{
        if let Some(id) = $ctx.cur_id {
            Some((id, DirtyField::Style))
        } else {
            None
        }
    }};
    (SetWidth, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $dt:expr, $v:expr) => {{
        Some(($id, DirtyField::Size | DirtyField::Layout))
    }};
    (SetHeight, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $dt:expr, $v:expr) => {{
        Some(($id, DirtyField::Size | DirtyField::Layout))
    }};
    (SetWidthCompact, $ctx:ident, $offset:ident, $payload:ident, $dt:expr, $v:expr) => {{
        if let Some(id) = $ctx.cur_id {
            Some((id, DirtyField::Size | DirtyField::Layout))
        } else {
            None
        }
    }};
    (SetHeightCompact, $ctx:ident, $offset:ident, $payload:ident, $dt:expr, $v:expr) => {{
        if let Some(id) = $ctx.cur_id {
            Some((id, DirtyField::Size | DirtyField::Layout))
        } else {
            None
        }
    }};
    (SetFlexDirection, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $dir:expr) => {{
        Some(($id, DirtyField::Layout))
    }};
    (SetJustifyContent, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $j:expr) => {{
        Some(($id, DirtyField::Layout))
    }};
    (SetAlignItems, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $a:expr) => {{
        Some(($id, DirtyField::Layout))
    }};
    (SetFlexWrap, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $w:expr) => {{
        Some(($id, DirtyField::Layout))
    }};
    (SetAlignContent, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $ac:expr) => {{
        Some(($id, DirtyField::Layout))
    }};
    (SetFlexGrow, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $grow:expr) => {{
        Some(($id, DirtyField::Layout))
    }};
    (SetPadding, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $t:expr, $r:expr, $b:expr, $l:expr) => {{
        Some(($id, DirtyField::Layout))
    }};
    (SetZIndex, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $z:expr) => {{
        Some(($id, DirtyField::Style))
    }};
    (SetFontSize, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $size:expr) => {{
        Some(($id, DirtyField::Text))
    }};
    (SetBorderRadius, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $r:expr) => {{
        Some(($id, DirtyField::Style))
    }};
    (AttachClick, $ctx:ident, $offset:ident, $payload:ident, $id:expr) => {{
        Some(($id, DirtyField::None))
    }};
    (AddChild, $ctx:ident, $offset:ident, $payload:ident, $pid:expr, $cid:expr) => {{
        Some(($pid, DirtyField::Children))
    }};
    (SetText, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $len_u32:expr) => {{
        let len = $len_u32 as usize;
        if $offset + len <= $payload.len() {
            $offset += len; // Skip text bytes
        }
        Some(($id, DirtyField::Text))
    }};
    (SetTextContent, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $len_u32:expr) => {{
        let len = $len_u32 as usize;
        if $offset + len <= $payload.len() {
            $offset += len;
        }
        Some(($id, DirtyField::Text))
    }};
    (SetTextColor, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $r:expr, $g:expr, $b:expr, $a:expr) => {{
        Some(($id, DirtyField::Style))
    }};
    (SetTextWeight, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $weight:expr) => {{
        Some(($id, DirtyField::Text))
    }};
    (SetTextFontFamily, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $len_u32:expr) => {{
        let len = $len_u32 as usize;
        if $offset + len <= $payload.len() {
            $offset += len;
        }
        Some(($id, DirtyField::Text))
    }};
    (SetViewType, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $vt:expr) => {{
        Some(($id, DirtyField::None))
    }};
    (SetLabel, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $len_u32:expr) => {{
        let len = $len_u32 as usize;
        if $offset + len <= $payload.len() {
            $offset += len;
        }
        None
    }};
    (SetSemantics, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $role:expr) => {{
        None
    }};
    (UpdateLayout, $ctx:ident, $offset:ident, $payload:ident) => {{
        None
    }};

    // === Transaction Operations ===
    (BeginTransaction, $ctx:ident, $offset:ident, $payload:ident, $seq_id:expr, $flags:expr) => {{
        TransactionOp::Begin($seq_id, $flags)
    }};
    (EndTransaction, $ctx:ident, $offset:ident, $payload:ident, $seq_id:expr) => {{
        TransactionOp::End($seq_id)
    }};
    (AbortTransaction, $ctx:ident, $offset:ident, $payload:ident, $seq_id:expr) => {{
        TransactionOp::Abort($seq_id)
    }};
    (SetNodeDirty, $ctx:ident, $offset:ident, $payload:ident, $id:expr, $fields:expr) => {{
        Some(($id, DirtyField::from_bits($fields)))
    }};

    // === LayoutRegistry Operations ===
    // These are read operations - no dirty tracking needed
    (GetLayout, $ctx:ident, $offset:ident, $payload:ident, $id:expr) => {{
        None
    }};
    (IsLayoutDirty, $ctx:ident, $offset:ident, $payload:ident, $id:expr) => {{
        None
    }};
    (ClearLayoutDirty, $ctx:ident, $offset:ident, $payload:ident, $id:expr) => {{
        None
    }};
    (GetLayoutBatch, $ctx:ident, $offset:ident, $payload:ident, $start_id:expr, $count:expr) => {{
        None
    }};

    // === Gesture Events - No dirty tracking ===
    (GestureTap, $ctx:ident, $offset:ident, $payload:ident, $node_id:expr, $x:expr, $y:expr) => {{
        None
    }};
    (GestureDoubleTap, $ctx:ident, $offset:ident, $payload:ident, $node_id:expr, $x:expr, $y:expr) => {{
        None
    }};
    (GestureLongPressStart, $ctx:ident, $offset:ident, $payload:ident, $node_id:expr, $x:expr, $y:expr) => {{
        None
    }};
    (GestureLongPressEnd, $ctx:ident, $offset:ident, $payload:ident, $node_id:expr, $x:expr, $y:expr) => {{
        None
    }};
    (GesturePanStart, $ctx:ident, $offset:ident, $payload:ident, $node_id:expr, $x:expr, $y:expr) => {{
        None
    }};
    (GesturePanUpdate, $ctx:ident, $offset:ident, $payload:ident, $node_id:expr, $x:expr, $y:expr, $dx:expr, $dy:expr) => {{
        None
    }};
    (GesturePanEnd, $ctx:ident, $offset:ident, $payload:ident, $node_id:expr, $x:expr, $y:expr, $vx:expr, $vy:expr) => {{
        None
    }};
    (GestureCancel, $ctx:ident, $offset:ident, $payload:ident, $node_id:expr) => {{
        None
    }};
    (UpdateDeviceInfo, $ctx:ident, $offset:ident, $payload:ident, $dpr:expr, $text_scale:expr, $width:expr, $height:expr, $safe_top:expr, $safe_bottom:expr, $platform:expr) => {{
        None
    }};
}

#[allow(dead_code)]
enum TransactionOp {
    Begin(u32, u16),
    End(u32),
    Abort(u32),
    None,
}

impl From<Option<(u32, DirtyField)>> for TransactionOp {
    fn from(_opt: Option<(u32, DirtyField)>) -> Self {
        TransactionOp::None
    }
}

/// Apply a staged command using saved node_id for compact operations
fn apply_staged_command(state: &mut SharedState, cmd: &StagedCommand, ctx: &mut CommandContext) {
    // Update ctx.cur_id for commands that need it
    match cmd.opcode {
        OpCode::CreateNode | OpCode::CreateTextNode | OpCode::SelectNode => {
            ctx.cur_id = Some(cmd.node_id);
        }
        _ => {}
    }

    // For compact ops, use the staged node_id instead of ctx.cur_id
    match cmd.opcode {
        OpCode::CreateNode => {
            state.create_node(cmd.node_id);
        }
        OpCode::CreateTextNode => {
            state.create_text_node(cmd.node_id);
        }
        OpCode::SetColorCompact => {
            if cmd.payload.len() >= 4 {
                let r = cmd.payload[0];
                let g = cmd.payload[1];
                let b = cmd.payload[2];
                let a = cmd.payload[3];
                state.set_color_rgba(cmd.node_id, r, g, b, a);
            }
        }
        OpCode::SetWidthCompact => {
            if cmd.payload.len() >= 5 {
                let dt = cmd.payload[0];
                let v = f32::from_le_bytes([
                    cmd.payload[1],
                    cmd.payload[2],
                    cmd.payload[3],
                    cmd.payload[4],
                ]);
                state.set_width(cmd.node_id, dt as u32, v);
            } else {
            }
        }
        OpCode::SetHeightCompact => {
            if cmd.payload.len() >= 5 {
                let dt = cmd.payload[0];
                let v = f32::from_le_bytes([
                    cmd.payload[1],
                    cmd.payload[2],
                    cmd.payload[3],
                    cmd.payload[4],
                ]);
                state.set_height(cmd.node_id, dt as u32, v);
            }
        }
        // All other ops use the standard handler
        _ => {
            apply_command_immediate(state, &cmd.opcode, &cmd.payload, ctx);
        }
    }
}

/// Apply a single command to shared state immediately (for non-transaction mode)
fn apply_command_immediate(
    state: &mut SharedState,
    opcode: &OpCode,
    payload: &[u8],
    ctx: &mut CommandContext,
) {
    // This is called when applying committed transactions or for backward compatibility
    match opcode {
        OpCode::CreateNode => {
            if payload.len() >= 4 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                state.create_node(id);
                ctx.cur_id = Some(id);
            }
        }
        OpCode::CreateTextNode => {
            if payload.len() >= 4 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                state.create_text_node(id);
                ctx.cur_id = Some(id);
            }
        }
        OpCode::SetViewType => {
            if let Some(id) = ctx.cur_id {
                if payload.len() >= 4 {
                    let vt = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    state.set_view_type(id, vt);
                }
            }
        }
        OpCode::SetColor => {
            if payload.len() >= 7 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let r = payload[4];
                let g = payload[5];
                let b = payload[6];
                state.set_color(id, r, g, b);
            }
        }
        OpCode::SetColorCompact => {
            if payload.len() >= 4 {
                if let Some(id) = ctx.cur_id {
                    let r = payload[0];
                    let g = payload[1];
                    let b = payload[2];
                    let a = payload[3];
                    state.set_color_rgba(id, r, g, b, a);
                }
            }
        }
        OpCode::SetWidth => {
            if payload.len() >= 9 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let dt = payload[4];
                let v = f32::from_le_bytes([payload[5], payload[6], payload[7], payload[8]]);
                state.set_width(id, dt as u32, v);
            }
        }
        OpCode::SetHeight => {
            if payload.len() >= 9 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let dt = payload[4];
                let v = f32::from_le_bytes([payload[5], payload[6], payload[7], payload[8]]);
                state.set_height(id, dt as u32, v);
            }
        }
        OpCode::SetWidthCompact => {
            if payload.len() >= 5 {
                if let Some(id) = ctx.cur_id {
                    let dt = payload[0];
                    let v = f32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
                    state.set_width(id, dt as u32, v);
                }
            }
        }
        OpCode::SetHeightCompact => {
            if payload.len() >= 5 {
                if let Some(id) = ctx.cur_id {
                    let dt = payload[0];
                    let v = f32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
                    state.set_height(id, dt as u32, v);
                }
            }
        }
        OpCode::SetFlexDirection => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let dir = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_flex_direction(id, dir);
            }
        }
        OpCode::SetJustifyContent => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let j = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_justify_content(id, j);
            }
        }
        OpCode::SetAlignItems => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let a = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_align_items(id, a);
            }
        }
        OpCode::SetFlexWrap => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let w = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_flex_wrap(id, w);
            }
        }
        OpCode::SetAlignContent => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let ac = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_align_content(id, ac);
            }
        }
        OpCode::SetFlexGrow => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let grow = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_flex_grow(id, grow);
            }
        }
        OpCode::SetZIndex => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let z = i32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_z_index(id, z);
            }
        }
        OpCode::SetFontSize => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let size = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_font_size(id, size);
            }
        }
        OpCode::SetBorderRadius => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let r = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_border_radius(id, r);
            }
        }
        OpCode::SetPadding => {
            if payload.len() >= 20 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let t = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let r = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                let b = f32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]);
                let l = f32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]);
                state.set_padding(id, t, r, b, l);
            }
        }
        OpCode::AttachClick => {
            if payload.len() >= 4 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                state.attach_click(id);
            }
        }
        OpCode::SetText => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let len =
                    u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
                let text_start = 8;
                let text_end = text_start + len;
                if text_end <= payload.len() {
                    let text = String::from_utf8_lossy(&payload[text_start..text_end]).to_string();
                    state.set_text(id, text);
                }
            }
        }
        OpCode::AddChild => {
            if payload.len() >= 8 {
                let pid = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let cid = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.add_child(pid, cid);
            }
        }
        OpCode::SelectNode => {
            if payload.len() >= 4 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                ctx.cur_id = Some(id);
            }
        }
        OpCode::SetTextContent => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let len =
                    u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
                let text_start = 8;
                let text_end = text_start + len;
                if text_end <= payload.len() {
                    let text = String::from_utf8_lossy(&payload[text_start..text_end]).to_string();
                    state.set_text(id, text);
                }
            }
        }
        OpCode::SetTextColor => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let r = payload[4];
                let g = payload[5];
                let b = payload[6];
                let a = payload[7];
                state.set_text_color(id, r, g, b, a);
            }
        }
        OpCode::SetTextWeight => {
            if payload.len() >= 6 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let weight = u16::from_le_bytes([payload[4], payload[5]]);
                state.set_font_weight(id, weight);
            }
        }
        OpCode::SetTextFontFamily => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let len =
                    u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
                let family_start = 8;
                let family_end = family_start + len;
                if family_end <= payload.len() {
                    let family =
                        String::from_utf8_lossy(&payload[family_start..family_end]).to_string();
                    state.set_font_family(id, family);
                }
            }
        }
        OpCode::SetNodeDirty => {
            if payload.len() >= 5 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let fields = payload[4];
                state.set_node_dirty(id, fields);
            }
        }
        // === Gesture Events - Host → WASM ===
        // These commands are generated by the GestureArena in Host layer
        // and dispatched to WASM for handling
        OpCode::GestureTap => {
            if payload.len() >= 12 {
                let node_id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let x = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let y = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                // Mark as handled - actual callback happens via WASM reading the command stream
                log::debug!("Host: GestureTap node={} pos=({:.1},{:.1})", node_id, x, y);
            }
        }
        OpCode::GestureDoubleTap => {
            if payload.len() >= 12 {
                let node_id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let x = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let y = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                log::debug!(
                    "Host: GestureDoubleTap node={} pos=({:.1},{:.1})",
                    node_id,
                    x,
                    y
                );
            }
        }
        OpCode::GestureLongPressStart => {
            if payload.len() >= 12 {
                let node_id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let x = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let y = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                log::debug!(
                    "Host: GestureLongPressStart node={} pos=({:.1},{:.1})",
                    node_id,
                    x,
                    y
                );
            }
        }
        OpCode::GestureLongPressEnd => {
            if payload.len() >= 12 {
                let node_id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let x = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let y = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                log::debug!(
                    "Host: GestureLongPressEnd node={} pos=({:.1},{:.1})",
                    node_id,
                    x,
                    y
                );
            }
        }
        OpCode::GesturePanStart => {
            if payload.len() >= 12 {
                let node_id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let x = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let y = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                log::debug!(
                    "Host: GesturePanStart node={} pos=({:.1},{:.1})",
                    node_id,
                    x,
                    y
                );
            }
        }
        OpCode::GesturePanUpdate => {
            if payload.len() >= 20 {
                let node_id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let x = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let y = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                let delta_x =
                    f32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]);
                let delta_y =
                    f32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]);
                log::debug!(
                    "Host: GesturePanUpdate node={} pos=({:.1},{:.1}) delta=({:.1},{:.1})",
                    node_id,
                    x,
                    y,
                    delta_x,
                    delta_y
                );
            }
        }
        OpCode::GesturePanEnd => {
            if payload.len() >= 20 {
                let node_id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let x = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let y = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                let velocity_x =
                    f32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]);
                let velocity_y =
                    f32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]);
                log::debug!(
                    "Host: GesturePanEnd node={} pos=({:.1},{:.1}) velocity=({:.1},{:.1})",
                    node_id,
                    x,
                    y,
                    velocity_x,
                    velocity_y
                );
            }
        }
        OpCode::GestureCancel => {
            if payload.len() >= 4 {
                let node_id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                log::debug!("Host: GestureCancel node={}", node_id);
            }
        }
        // === Gesture Handler Registration ===
        // Unified tap handler - count determines single/double/triple/etc (1-N)
        OpCode::RegisterTapHandler => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let count = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                log::info!("RegisterTapHandler for node {} with count {}", id, count);
                get_handler_registry()
                    .lock()
                    .unwrap()
                    .register(id, HandlerType::Tap(count));
            }
        }
        OpCode::RegisterLongPressHandler => {
            if payload.len() >= 4 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                log::info!("RegisterLongPressHandler for node {}", id);
                get_handler_registry()
                    .lock()
                    .unwrap()
                    .register(id, HandlerType::LongPress);
            }
        }
        OpCode::RegisterPanHandler => {
            if payload.len() >= 4 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                log::info!("RegisterPanHandler for node {}", id);
                get_handler_registry()
                    .lock()
                    .unwrap()
                    .register(id, HandlerType::Pan);
            }
        }
        // Note: RegisterDoubleTapHandler and RegisterMultiTapHandler removed
        // All tap gestures now use unified RegisterTapHandler with count parameter
        OpCode::UnregisterGestureHandler => {
            if payload.len() >= 4 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                get_handler_registry().lock().unwrap().unregister(id);
            }
        }
        // === Unified Gesture Registration (Phase 1) ===
        OpCode::RegisterGesture => {
            if payload.len() >= 6 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let mask = u16::from_le_bytes([payload[4], payload[5]]);
                log::info!("RegisterGesture for node {} with mask {:#b}", id, mask);
                get_handler_registry()
                    .lock()
                    .unwrap()
                    .register_by_mask(id, mask);
            }
        }
        OpCode::SetGestureConfig => {
            if payload.len() >= 9 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let config_type = payload[4];
                let value = u32::from_le_bytes([payload[5], payload[6], payload[7], payload[8]]);
                log::info!(
                    "SetGestureConfig for node {}: type={} value={}",
                    id,
                    config_type,
                    value
                );
                get_handler_registry()
                    .lock()
                    .unwrap()
                    .set_config(id, config_type, value);
            }
        }
        OpCode::UpdateDeviceInfo => {
            if payload.len() >= 28 {
                let dpr = f32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let text_scale =
                    f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let width = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                let height =
                    f32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]);
                let _safe_top =
                    f32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]);
                let _safe_bottom =
                    f32::from_le_bytes([payload[20], payload[21], payload[22], payload[23]]);
                let platform =
                    u32::from_le_bytes([payload[24], payload[25], payload[26], payload[27]]);
                log::info!("Host: UpdateDeviceInfo dpr={:.2}, text_scale={:.2}, size={:.0}x{:.0}, platform={}",
                    dpr, text_scale, width, height, platform);
            }
        }
        // === Layer Effects (92-96) - Vello Native Layer Rendering ===
        OpCode::SetOpacity => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let opacity = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_opacity(id, opacity);
            }
        }
        OpCode::SetShadow => {
            if payload.len() >= 20 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let offset_x = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let offset_y =
                    f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                let blur = f32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]);
                let color =
                    u32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]);
                state.set_shadow(id, offset_x, offset_y, blur, color);
            }
        }
        OpCode::SetBlur => {
            if payload.len() >= 8 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let radius = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                state.set_blur(id, radius);
            }
        }
        OpCode::SetClipToBounds => {
            if payload.len() >= 5 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let clip = payload[4] != 0;
                state.set_clip_to_bounds(id, clip);
            }
        }
        OpCode::SetPosition => {
            if payload.len() >= 12 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let x = f32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let y = f32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
                state.set_position(id, x, y);
            }
        }
        OpCode::SetBlurStyle => {
            if payload.len() >= 5 {
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let style = payload[4];
                state.set_blur_style(id, style);
            }
        }
        _ => {
            // Other opcodes are handled during transaction processing
        }
    }
}

/// Process command stream with transaction support
fn process_command_stream_with_tx(
    state: &mut SharedState,
    command_data: &[u8],
    tx_processor: &mut TransactionProcessor,
) -> anyhow::Result<()> {
    let mut offset = 0;
    let mut ctx = CommandContext::new();

    while offset < command_data.len() {
        let op_byte = command_data[offset];
        offset += 1;

        let op = match OpCode::from_u8(op_byte) {
            Some(o) => o,
            None => {
                log::warn!("Unknown opcode: {}", op_byte);
                continue;
            }
        };

        // Handle transaction operations specially
        match op {
            OpCode::BeginTransaction => {
                if offset + 6 <= command_data.len() {
                    let seq_id = u32::from_le_bytes([
                        command_data[offset],
                        command_data[offset + 1],
                        command_data[offset + 2],
                        command_data[offset + 3],
                    ]);
                    let flags =
                        u16::from_le_bytes([command_data[offset + 4], command_data[offset + 5]]);
                    offset += 6;

                    if let Err(e) = tx_processor.begin(seq_id, flags) {
                        log::warn!("[TX] Failed to begin transaction: {}", e);
                    }
                }
                continue;
            }
            OpCode::EndTransaction => {
                if offset + 4 <= command_data.len() {
                    let seq_id = u32::from_le_bytes([
                        command_data[offset],
                        command_data[offset + 1],
                        command_data[offset + 2],
                        command_data[offset + 3],
                    ]);
                    offset += 4;

                    if let Ok(commands) = tx_processor.commit(seq_id) {
                        // Apply committed commands immediately
                        for cmd in &commands {
                            apply_staged_command(state, cmd, &mut ctx);
                        }
                    } else {
                        log::warn!("[TX] Failed to commit transaction");
                    }
                }
                continue;
            }
            OpCode::AbortTransaction => {
                if offset + 4 <= command_data.len() {
                    let seq_id = u32::from_le_bytes([
                        command_data[offset],
                        command_data[offset + 1],
                        command_data[offset + 2],
                        command_data[offset + 3],
                    ]);
                    offset += 4;

                    if let Err(e) = tx_processor.abort(seq_id) {
                        log::warn!("Failed to abort transaction: {}", e);
                    }
                }
                continue;
            }
            // === LayoutRegistry Operations - direct read, no transaction ===
            OpCode::GetLayout
            | OpCode::IsLayoutDirty
            | OpCode::ClearLayoutDirty
            | OpCode::GetLayoutBatch => {
                // LayoutRegistry ops are read-only queries, skip transaction staging
                // The actual read happens via shared memory layout_results area
                // No Host-side processing needed - WASM reads directly from memory
                let base_len = op.data_len();
                if offset + base_len <= command_data.len() {
                    offset += base_len;
                }
                continue;
            }
            // === Gesture Events - Host to WASM, no transaction ===
            OpCode::GestureTap
            | OpCode::GestureDoubleTap
            | OpCode::GestureLongPressStart
            | OpCode::GestureLongPressEnd
            | OpCode::GesturePanStart
            | OpCode::GesturePanUpdate
            | OpCode::GesturePanEnd
            | OpCode::GestureCancel => {
                // Gesture events are one-way notifications from Host to WASM
                // They don't modify state, so skip transaction staging
                let base_len = op.data_len();
                if offset + base_len <= command_data.len() {
                    offset += base_len;
                }
                continue;
            }
            OpCode::UpdateDeviceInfo => {
                // Device info is passed through to WASM
                let base_len = op.data_len();
                if offset + base_len <= command_data.len() {
                    offset += base_len;
                }
                continue;
            }
            _ => {}
        }

        // For non-transaction ops, extract payload and stage or apply
        let base_len = op.data_len();

        // Handle variable-length opcodes (text content)
        let actual_len = match op {
            OpCode::SetText
            | OpCode::SetTextContent
            | OpCode::SetTextFontFamily
            | OpCode::SetLabel => {
                // Read the len field (u32 at offset 4)
                if offset + 8 <= command_data.len() {
                    let text_len = u32::from_le_bytes([
                        command_data[offset + 4],
                        command_data[offset + 5],
                        command_data[offset + 6],
                        command_data[offset + 7],
                    ]) as usize;
                    base_len + text_len
                } else {
                    base_len
                }
            }
            _ => base_len,
        };

        let payload_end = offset + actual_len;

        if payload_end > command_data.len() {
            log::warn!(
                "Command payload out of bounds: opcode={:?}, need={}, have={}",
                op,
                actual_len,
                command_data.len() - offset
            );
            break;
        }

        let payload = &command_data[offset..payload_end];
        offset = payload_end;

        // Extract node_id from payload for CreateNode/CreateTextNode/SelectNode
        match op {
            OpCode::CreateNode | OpCode::CreateTextNode | OpCode::SelectNode => {
                if payload.len() >= 4 {
                    let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    ctx.cur_id = Some(id);
                }
            }
            _ => {}
        }

        // Stage command if transaction active, else apply immediately
        match tx_processor.state {
            TransactionState::Active { .. } => {
                // Determine node_id: use cur_id for compact ops, extract from payload for others
                let node_id = match op {
                    OpCode::CreateNode | OpCode::CreateTextNode | OpCode::SelectNode => {
                        ctx.cur_id.unwrap_or(0)
                    }
                    OpCode::SetColorCompact
                    | OpCode::SetWidthCompact
                    | OpCode::SetHeightCompact => ctx.cur_id.unwrap_or(0),
                    _ => {
                        // Extract from payload (first 4 bytes)
                        if payload.len() >= 4 {
                            u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]])
                        } else {
                            ctx.cur_id.unwrap_or(0)
                        }
                    }
                };

                let dirty_fields = get_dirty_field_for_opcode(&op);

                let staged = StagedCommand {
                    opcode: op,
                    node_id,
                    payload: payload.to_vec(),
                    dirty_fields,
                };

                let _ = tx_processor.stage_command(staged);
            }
            _ => {
                // No active transaction, apply immediately
                apply_command_immediate(state, &op, payload, &mut ctx);
            }
        }
    }

    Ok(())
}

/// Legacy process function for backward compatibility
#[cfg(test)]
fn _process_command_stream_inner(
    state: &mut SharedState,
    command_data: &[u8],
) -> anyhow::Result<()> {
    let mut tx_processor = TransactionProcessor::new();
    process_command_stream_with_tx(state, command_data, &mut tx_processor)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn process_command_stream(
    state: &Arc<Mutex<SharedState>>,
    command_data: &[u8],
) -> anyhow::Result<()> {
    let mut s = state.lock().unwrap();
    let mut tx_processor = get_tx_processor().lock().unwrap();
    process_command_stream_with_tx(&mut *s, command_data, &mut *tx_processor)
}

#[cfg(target_arch = "wasm32")]
pub fn process_command_stream(
    state: &std::cell::RefCell<SharedState>,
    command_data: &[u8],
) -> anyhow::Result<()> {
    let mut s = state.borrow_mut();
    let mut tx_processor = TransactionProcessor::new();
    process_command_stream_with_tx(&mut *s, command_data, &mut tx_processor)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn process_commands(
    memory: &mut [u8],
    buffer_ptr: u32,
    state: &Arc<Mutex<SharedState>>,
) -> anyhow::Result<()> {
    let bs = buffer_ptr as usize;
    if bs + 4 > memory.len() {
        return Err(anyhow::anyhow!(
            "WASM memory out of bounds reading command_len"
        ));
    }
    let clen = u32::from_le_bytes(memory[bs..bs + 4].try_into()?);

    if clen == 0 {
        return Ok(());
    }

    let data_start = bs + 16;
    let data_end = data_start + clen as usize;
    if data_end > memory.len() {
        return Err(anyhow::anyhow!(
            "WASM memory out of bounds reading command_data"
        ));
    }

    let mut s = state.lock().unwrap();
    let mut tx_processor = get_tx_processor().lock().unwrap();
    let result =
        process_command_stream_with_tx(&mut *s, &memory[data_start..data_end], &mut *tx_processor);

    // Clear command buffer
    memory[bs..bs + 4].copy_from_slice(&0u32.to_le_bytes());

    result
}

#[cfg(target_arch = "wasm32")]
pub fn process_commands(
    memory: &mut [u8],
    buffer_ptr: u32,
    state: &std::cell::RefCell<SharedState>,
) -> anyhow::Result<()> {
    let bs = buffer_ptr as usize;
    if bs + 4 > memory.len() {
        return Err(anyhow::anyhow!(
            "WASM memory out of bounds reading command_len"
        ));
    }
    let clen = u32::from_le_bytes(memory[bs..bs + 4].try_into()?);
    if clen == 0 {
        return Ok(());
    }
    let data_start = bs + 16;
    let data_end = data_start + clen as usize;
    if data_end > memory.len() {
        return Err(anyhow::anyhow!(
            "WASM memory out of bounds reading command_data"
        ));
    }
    let mut s = state.borrow_mut();
    let mut tx_processor = TransactionProcessor::new();
    let _ =
        process_command_stream_with_tx(&mut *s, &memory[data_start..data_end], &mut tx_processor);
    memory[bs..bs + 4].copy_from_slice(&0u32.to_le_bytes());
    Ok(())
}

pub fn sync_layout_to_wasm(
    memory: &mut [u8],
    buffer_ptr: u32,
    state: &SharedState,
) -> anyhow::Result<()> {
    let bs = buffer_ptr as usize;
    let ls = bs + 16 + MAX_COMMAND_BYTES;
    let ms = ls + (MAX_NODES * 16);
    let total_required = ms + (MAX_NODES / 32 * 4);
    if total_required > memory.len() {
        return Err(anyhow::anyhow!("WASM memory too small for layout buffer"));
    }

    // Mark nodes as dirty that were registered by Render thread after layout computation
    #[cfg(not(target_arch = "wasm32"))]
    {
        let layout_dirty_nodes = dyxel_shared::layout_sync::take_layout_dirty_nodes();
        if !layout_dirty_nodes.is_empty() {
            let mut tx = get_tx_processor().lock().unwrap();
            for id in layout_dirty_nodes {
                tx.dirty_tracker.mark_dirty(id, DirtyField::Layout);
            }
        }
    }

    // Get dirty tracker for this sync
    #[cfg(not(target_arch = "wasm32"))]
    let _dirty_tracker_opt = get_dirty_tracker();
    #[cfg(target_arch = "wasm32")]
    let _dirty_tracker_opt: Option<DirtyTracker> = None;

    // Build parent -> children mapping for topological traversal
    let mut children_map: std::collections::HashMap<u32, Vec<u32>> =
        std::collections::HashMap::new();
    let mut root_nodes = Vec::new();
    for (&id, node) in &state.nodes {
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
        if id as usize >= MAX_NODES {
            continue;
        }

        if let Some(node) = state.nodes.get(&id) {
            // Always calculate absolute position for this node (needed by children)
            let (parent_abs_x, parent_abs_y) = abs_positions
                .get(&node.parent_id)
                .copied()
                .unwrap_or((0.0, 0.0));

            // Always get layout and calculate absolute position (needed for both dirty and clean nodes)
            if let Ok(layout) = state.taffy.layout(node.taffy_node) {
                // Skip zero-size nodes (not yet computed)
                if layout.size.width <= 0.0 || layout.size.height <= 0.0 {
                    continue;
                }

                // Calculate absolute position
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                abs_positions.insert(id, (abs_x, abs_y));

                // Always check if layout changed and update shared buffer
                // This is needed because parent layout changes affect child absolute positions
                let target = ls + (id as usize * 16);
                let nx = abs_x.to_le_bytes();
                let ny = abs_y.to_le_bytes();
                let nw = layout.size.width.to_le_bytes();
                let nh = layout.size.height.to_le_bytes();
                let changed = memory[target..target + 4] != nx
                    || memory[target + 4..target + 8] != ny
                    || memory[target + 8..target + 12] != nw
                    || memory[target + 12..target + 16] != nh;
                if changed {
                    memory[target..target + 4].copy_from_slice(&nx);
                    memory[target + 4..target + 8].copy_from_slice(&ny);
                    memory[target + 8..target + 12].copy_from_slice(&nw);
                    memory[target + 12..target + 16].copy_from_slice(&nh);
                    let word_idx = (id / 32) as usize;
                    let bit_idx = id % 32;
                    let mask_pos = ms + (word_idx * 4);
                    let mut mask = u32::from_le_bytes(memory[mask_pos..mask_pos + 4].try_into()?);
                    mask |= 1 << bit_idx;
                    memory[mask_pos..mask_pos + 4].copy_from_slice(&mask.to_le_bytes());
                }

                // Always process children (they may need updated parent positions)
                if let Some(children) = children_map.get(&id) {
                    for &child in children {
                        queue.push_back(child);
                    }
                }
            }
        }
    }

    // Clear dirty tracker after sync
    #[cfg(not(target_arch = "wasm32"))]
    clear_dirty_tracker();

    Ok(())
}
