// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::{Arc, Mutex};
use dyxel_shared::{OpCode, MAX_COMMAND_BYTES, MAX_NODES};
use crate::state::SharedState;

macro_rules! handle_op {
    (CreateNode, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr) => {
        $s.create_node($id); $cur_id = Some($id);
    };
    (SetViewType, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $vt:expr) => {
        $s.set_view_type($id, $vt);
    };
    (SetColor, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $r:expr, $g:expr, $b:expr) => {
        $s.set_color($id, $r, $g, $b);
    };
    (SetWidth, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $dt:expr, $v:expr) => {
        $s.set_width($id, $dt as u32, $v);
    };
    (SetHeight, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $dt:expr, $v:expr) => {
        $s.set_height($id, $dt as u32, $v);
    };
    (SetFlexDirection, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $dir:expr) => {
        $s.set_flex_direction($id, $dir);
    };
    (SetJustifyContent, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $j:expr) => {
        $s.set_justify_content($id, $j);
    };
    (SetAlignItems, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $a:expr) => {
        $s.set_align_items($id, $a);
    };
    (SetPosition, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $p:expr) => {
        $s.set_position($id, $p);
    };
    (SetInset, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $t:expr, $r:expr, $b:expr, $l:expr) => {
        $s.set_inset($id, $t, $r, $b, $l);
    };
    (SetFlexGrow, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $grow:expr) => {
        $s.set_flex_grow($id, $grow);
    };
    (SetZIndex, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $z:expr) => {
        $s.set_z_index($id, $z);
    };
    (SetFontSize, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $size:expr) => {
        $s.set_font_size($id, $size);
    };
    (SetBorderRadius, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $r:expr) => {
        $s.set_border_radius($id, $r);
    };
    (SetPadding, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $t:expr, $r:expr, $b:expr, $l:expr) => {
        $s.set_padding($id, $t, $r, $b, $l);
    };
    (AttachClick, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr) => {
        $s.attach_click($id);
    };
    (SetText, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $len_u32:expr) => {
        let len = $len_u32 as usize;
        if $offset + len <= $command_data.len() {
            let text = String::from_utf8_lossy(&$command_data[$offset..$offset+len]).to_string();
            $s.set_text($id, text);
            $offset += len;
        }
    };
    (SetLabel, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $len_u32:expr) => {
        let len = $len_u32 as usize;
        if $offset + len <= $command_data.len() { $offset += len; }
    };
    (AddChild, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $pid:expr, $cid:expr) => {
        $s.add_child($pid, $cid);
    };
    (SelectNode, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr) => {
        $cur_id = Some($id);
    };
    (SetColorCompact, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $r:expr, $g:expr, $b:expr, $a:expr) => {
        if let Some(id) = $cur_id { $s.set_color(id, $r, $g, $b); }
    };
    (SetWidthCompact, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $dt:expr, $v:expr) => {
        if let Some(id) = $cur_id { $s.set_width(id, $dt as u32, $v); }
    };
    (SetHeightCompact, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $dt:expr, $v:expr) => {
        if let Some(id) = $cur_id { $s.set_height(id, $dt as u32, $v); }
    };
    (UpdateLayout, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident) => {};
    (SetSemantics, $s:ident, $cur_id:ident, $offset:ident, $command_data:ident, $id:expr, $role:expr) => {};
}

// Internal implementation function
fn process_command_stream_inner(state: &mut SharedState, command_data: &[u8]) -> anyhow::Result<()> {
    let mut offset = 0; 
    let mut cur_id: Option<u32> = None;
    while offset < command_data.len() {
        let op_byte = command_data[offset]; 
        offset += 1;
        let op = match OpCode::from_u8(op_byte) {
            Some(o) => o,
            None => { log::warn!("Unknown opcode: {}", op_byte); continue; }
        };
        dyxel_shared::dispatch_op!(op, command_data, offset, handle_op, state, cur_id, offset, command_data);
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn process_command_stream(state: &Arc<Mutex<SharedState>>, command_data: &[u8]) -> anyhow::Result<()> {
    let mut s = state.lock().unwrap(); 
    process_command_stream_inner(&mut *s, command_data)
}

#[cfg(target_arch = "wasm32")]
pub fn process_command_stream(state: &std::cell::RefCell<SharedState>, command_data: &[u8]) -> anyhow::Result<()> {
    let mut s = state.borrow_mut();
    process_command_stream_inner(&mut *s, command_data)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn process_commands(memory: &mut [u8], buffer_ptr: u32, state: &Arc<Mutex<SharedState>>) -> anyhow::Result<()> {
    let bs = buffer_ptr as usize; 
    if bs + 4 > memory.len() {
        return Err(anyhow::anyhow!("WASM memory out of bounds reading command_len"));
    }
    let clen = u32::from_le_bytes(memory[bs..bs+4].try_into()?);
    if clen == 0 { return Ok(()); }
    let data_start = bs + 16;
    let data_end = data_start + clen as usize;
    if data_end > memory.len() {
        return Err(anyhow::anyhow!("WASM memory out of bounds reading command_data. Length: {}, Buffer End: {}", clen, memory.len()));
    }
    let mut s = state.lock().unwrap();
    let _ = process_command_stream_inner(&mut *s, &memory[data_start .. data_end]);
    memory[bs..bs + 4].copy_from_slice(&0u32.to_le_bytes()); 
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn process_commands(memory: &mut [u8], buffer_ptr: u32, state: &std::cell::RefCell<SharedState>) -> anyhow::Result<()> {
    let bs = buffer_ptr as usize; 
    if bs + 4 > memory.len() {
        return Err(anyhow::anyhow!("WASM memory out of bounds reading command_len"));
    }
    let clen = u32::from_le_bytes(memory[bs..bs+4].try_into()?);
    if clen == 0 { return Ok(()); }
    let data_start = bs + 16;
    let data_end = data_start + clen as usize;
    if data_end > memory.len() {
        return Err(anyhow::anyhow!("WASM memory out of bounds reading command_data. Length: {}, Buffer End: {}", clen, memory.len()));
    }
    let mut s = state.borrow_mut();
    let _ = process_command_stream_inner(&mut *s, &memory[data_start .. data_end]);
    memory[bs..bs + 4].copy_from_slice(&0u32.to_le_bytes()); 
    Ok(())
}

pub fn sync_layout_to_wasm(memory: &mut [u8], buffer_ptr: u32, state: &SharedState) -> anyhow::Result<()> {
    let bs = buffer_ptr as usize;
    let ls = bs + 16 + MAX_COMMAND_BYTES;
    let ms = ls + (MAX_NODES * 16); 
    let total_required = ms + (MAX_NODES / 32 * 4);
    if total_required > memory.len() {
        return Err(anyhow::anyhow!("WASM memory too small for layout buffer. Required: {}, Actual: {}", total_required, memory.len()));
    }
    for (&id, node) in &state.nodes {
        if id as usize >= MAX_NODES { continue; }
        if let Ok(layout) = state.taffy.layout(node.taffy_node) {
            let target = ls + (id as usize * 16);
            let nx = layout.location.x.to_le_bytes();
            let ny = layout.location.y.to_le_bytes();
            let nw = layout.size.width.to_le_bytes();
            let nh = layout.size.height.to_le_bytes();
            let changed = memory[target..target+4] != nx || 
                         memory[target+4..target+8] != ny ||
                         memory[target+8..target+12] != nw ||
                         memory[target+12..target+16] != nh;
            if changed {
                memory[target..target+4].copy_from_slice(&nx);
                memory[target+4..target+8].copy_from_slice(&ny);
                memory[target+8..target+12].copy_from_slice(&nw);
                memory[target+12..target+16].copy_from_slice(&nh);
                let word_idx = (id / 32) as usize;
                let bit_idx = id % 32;
                let mask_pos = ms + (word_idx * 4);
                let mut mask = u32::from_le_bytes(memory[mask_pos..mask_pos+4].try_into()?);
                mask |= 1 << bit_idx;
                memory[mask_pos..mask_pos+4].copy_from_slice(&mask.to_le_bytes());
            }
        }
    }
    Ok(())
}
