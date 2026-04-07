// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dyxel View Layer - WASM-side UI Building
//!
//! ## Logical Pixel System
//!
//! Default unit is logical pixels (LP). Use `px()` for physical pixels:

pub mod gesture;
pub use gesture::*;

// Re-export device pixel utilities
pub use dyxel_shared::{PxExt, LpExt, px, lp, SizeUnit, FontSizeUnit, DeviceInfo};

// Re-export futures-signals for reactive programming
pub use futures_signals::signal::{Signal, SignalExt, Mutable};
pub use futures_signals::signal_vec::SignalVecExt;

use std::sync::atomic::{AtomicU32, Ordering};
use std::cell::RefCell;
use std::collections::HashMap;
pub use dyxel_shared::{FlexDirection, JustifyContent, AlignItems, FlexWrap, AlignContent, Dimension, Role, ViewType, OpCode, LayoutResult, MAX_COMMAND_BYTES, SharedBuffer, DirtyField, TransactionFlags};
use dyxel_shared::push_command;

// Re-export RSX macro
pub use dyxel_rsx::rsx;

/// Panic info buffer for debugging WASM crashes
static mut PANIC_BUFFER: [u8; 256] = [0; 256];

static CLICK_COUNT: AtomicU32 = AtomicU32::new(0);
static EVENT_COUNT: AtomicU32 = AtomicU32::new(0);
static GESTURE_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn init_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        unsafe {
            let msg = if let Some(location) = info.location() {
                format!("{}:{}", location.file(), location.line())
            } else {
                "panic at unknown location".to_string()
            };
            let bytes = msg.as_bytes();
            let len = bytes.len().min(255);
            PANIC_BUFFER[0] = len as u8;
            for i in 0..len {
                PANIC_BUFFER[i + 1] = bytes[i];
            }
        }
    }));
}

#[unsafe(no_mangle)]
pub extern "C" fn dyxel_get_panic_ptr() -> u32 {
    (&raw const PANIC_BUFFER) as *const u8 as u32
}

pub mod dual_track_wasm;

#[unsafe(no_mangle)]
pub static mut SHARED_BUFFER: SharedBuffer = SharedBuffer {
    command_len: 0,
    max_node_id: 0,
    capacity: dyxel_shared::INITIAL_CAPACITY as u32,
    _padding: [0; 1],
    command_data: [0; MAX_COMMAND_BYTES],
    layout_results: [LayoutResult { x: 0.0, y: 0.0, width: 0.0, height: 0.0 }; dyxel_shared::MAX_CAPACITY],
    generations: [0; dyxel_shared::MAX_CAPACITY],
    dirty_mask: [0; 128],
    input_buffer: dyxel_shared::InputBuffer::new(),
    device_info: dyxel_shared::DeviceInfo {
        device_pixel_ratio: 1.0,
        text_scale_factor: 1.0,
        screen_width_lp: 375.0,
        screen_height_lp: 812.0,
        safe_area_top: 0.0,
        safe_area_bottom: 0.0,
        platform: 0,
        _padding: [0.0; 3],
    },
};

#[unsafe(no_mangle)]
pub extern "C" fn dyxel_get_protocol_hash() -> u64 {
    dyxel_shared::PROTOCOL_HASH
}

#[unsafe(no_mangle)]
pub extern "C" fn dyxel_get_shared_buffer_ptr() -> u32 {
    std::ptr::addr_of!(SHARED_BUFFER) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn dyxel_get_command_len() -> u32 {
    unsafe { SHARED_BUFFER.command_len }
}

static mut LAST_SELECTED_NODE: Option<u32> = None;

fn select_node(id: u32) {
    unsafe {
        if LAST_SELECTED_NODE == Some(id) { return; }
        push_command!(SHARED_BUFFER, SelectNode, id);
        LAST_SELECTED_NODE = Some(id);
    }
}

fn track_node(id: u32) { unsafe { if id > SHARED_BUFFER.max_node_id { SHARED_BUFFER.max_node_id = id; } } }
pub fn get_layout(id: u32) -> LayoutResult {
    unsafe {
        let len = (*std::ptr::addr_of!(SHARED_BUFFER.layout_results)).len();
        if (id as usize) < len {
            SHARED_BUFFER.layout_results[id as usize]
        } else {
            LayoutResult { x: 0.0, y: 0.0, width: 0.0, height: 0.0 }
        }
    }
}

pub fn hit_test(x: f32, y: f32) -> Option<u32> {
    let max_id = unsafe { SHARED_BUFFER.max_node_id };
    for id in (1..=max_id).rev() {
        let layout = get_layout(id);
        if layout.width > 0.0 && layout.height > 0.0 {
            if x >= layout.x && x <= layout.x + layout.width &&
               y >= layout.y && y <= layout.y + layout.height {
                return Some(id);
            }
        }
    }
    None
}

// ===== Gesture Command Processing =====

thread_local! {
    static EXECUTOR: RefCell<Vec<std::pin::Pin<Box<dyn futures_util::future::Future<Output = ()>>>>> = RefCell::new(Vec::new());
    static CLICK_HANDLERS: RefCell<HashMap<u32, Box<dyn FnMut()>>> = RefCell::new(HashMap::new());
    // Unified tap handlers - all tap counts (single, double, triple+) use this
    // Now supports multiple handlers per node for different tap counts
    static TAP_HANDLERS: RefCell<HashMap<u32, TapHandlerEntry>> = RefCell::new(HashMap::new());
    static LONG_PRESS_HANDLERS: RefCell<HashMap<u32, Box<dyn FnMut(GestureEvent)>>> = RefCell::new(HashMap::new());
    static PAN_HANDLERS: RefCell<HashMap<u32, Box<dyn FnMut(GestureEvent)>>> = RefCell::new(HashMap::new());
    // Scale gesture handlers
    static SCALE_HANDLERS: RefCell<HashMap<u32, Box<dyn FnMut(GestureEvent)>>> = RefCell::new(HashMap::new());
    // Rotation gesture handlers
    static ROTATION_HANDLERS: RefCell<HashMap<u32, Box<dyn FnMut(GestureEvent)>>> = RefCell::new(HashMap::new());
    // Shadow configuration storage (for partial shadow config via separate methods)
    static SHADOW_CONFIG: RefCell<HashMap<u32, ShadowConfig>> = RefCell::new(HashMap::new());
    // Note: PARENT_MAP removed - Host now handles event bubbling via HandlerRegistry
}

/// Entry for tap handlers - supports multiple callbacks for different tap counts
struct TapHandlerEntry {
    /// Handler for single tap (count = 1)
    single_tap: Option<Box<dyn FnMut(GestureEvent)>>,
    /// Handler for double tap (count = 2)
    double_tap: Option<Box<dyn FnMut(GestureEvent)>>,
    /// Handler for triple+ taps (count >= 3)
    multi_tap: Option<Box<dyn FnMut(GestureEvent)>>,
}

/// Shadow configuration for layer effects
#[derive(Clone, Copy, Debug, Default)]
struct ShadowConfig {
    offset_x: f32,
    offset_y: f32,
    blur: f32,
    color: u32, // RGBA
}

impl TapHandlerEntry {
    fn new() -> Self {
        Self {
            single_tap: None,
            double_tap: None,
            multi_tap: None,
        }
    }

    /// Dispatch event to the appropriate handler based on tap_count
    fn dispatch(&mut self, event: GestureEvent) {
        match event.tap_count {
            1 => {
                if let Some(handler) = &mut self.single_tap {
                    handler(event);
                }
            }
            2 => {
                if let Some(handler) = &mut self.double_tap {
                    handler(event);
                }
            }
            _ => {
                if let Some(handler) = &mut self.multi_tap {
                    handler(event);
                }
            }
        }
    }
}

fn process_gesture_commands() {
    use dyxel_shared::OpCode;
    
    let cmd_len = unsafe { SHARED_BUFFER.command_len as usize };
    if cmd_len == 0 { return; }
    
    let data = unsafe { &(&*(&raw const SHARED_BUFFER.command_data))[..cmd_len] };
    let mut offset = 0;
    
    while offset < data.len() {
        let op_byte = data[offset];
        offset += 1;
        
        let op = match OpCode::from_u8(op_byte) {
            Some(o) => o,
            None => {
                // Unknown opcode - skip remaining data to avoid infinite loop
                log(&format!("Warning: Unknown opcode {} at offset {}, skipping remaining {}", 
                    op_byte, offset, data.len() - offset));
                break;
            }
        };
        
        match op {
            OpCode::GestureTap => {
                if offset + 12 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 12;
                    dispatch_tap_with_bubble(node_id, x, y);
                    GESTURE_COUNT.fetch_add(1, Ordering::SeqCst);
                }
            }
            // Note: GestureDoubleTap removed - unified with GestureTap using tap_count
            OpCode::GestureLongPressStart => {
                if offset + 12 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 12;
                    LONG_PRESS_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) { f(GestureEvent::long_press(node_id, x, y, GesturePhase::Began)); }
                    });
                }
            }
            OpCode::GestureLongPressEnd => {
                if offset + 12 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 12;
                    LONG_PRESS_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) { f(GestureEvent::long_press(node_id, x, y, GesturePhase::Ended)); }
                    });
                }
            }
            OpCode::GesturePanStart => {
                if offset + 12 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 12;
                    PAN_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) { f(GestureEvent::pan(node_id, x, y, 0.0, 0.0, GesturePhase::Began)); }
                    });
                }
            }
            OpCode::GesturePanUpdate => {
                if offset + 20 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    let dx = f32::from_le_bytes([data[offset+12], data[offset+13], data[offset+14], data[offset+15]]);
                    let dy = f32::from_le_bytes([data[offset+16], data[offset+17], data[offset+18], data[offset+19]]);
                    offset += 20;
                    PAN_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) { f(GestureEvent::pan(node_id, x, y, dx, dy, GesturePhase::Changed)); }
                    });
                }
            }
            OpCode::GesturePanEnd => {
                if offset + 20 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 20;
                    PAN_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) {
                            f(GestureEvent::pan(node_id, x, y, 0.0, 0.0, GesturePhase::Ended));
                        }
                    });
                }
            }
            // === Direct Gesture Events (Host has already resolved bubbling) ===
            OpCode::DirectGestureTap => {
                if offset + 12 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 12;
                    // Direct call - no bubbling needed
                    TAP_HANDLERS.with(|h| {
                        if let Some(entry) = h.borrow_mut().get_mut(&node_id) {
                            entry.dispatch(GestureEvent::tap(node_id, x, y, 1));
                        }
                    });
                    GESTURE_COUNT.fetch_add(1, Ordering::SeqCst);
                }
            }
            // Note: DirectGestureDoubleTap removed - unified with DirectGestureTap using tap_count
            OpCode::DirectGestureLongPress => {
                if offset + 12 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 12;
                    LONG_PRESS_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) { f(GestureEvent::long_press(node_id, x, y, GesturePhase::Began)); }
                    });
                }
            }
            OpCode::DirectGestureLongPressEnd => {
                if offset + 12 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 12;
                    LONG_PRESS_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) { f(GestureEvent::long_press(node_id, x, y, GesturePhase::Ended)); }
                    });
                }
            }
            OpCode::DirectGesturePanStart => {
                if offset + 12 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 12;
                    PAN_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) { f(GestureEvent::pan(node_id, x, y, 0.0, 0.0, GesturePhase::Began)); }
                    });
                }
            }
            OpCode::DirectGesturePanUpdate => {
                if offset + 20 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    let dx = f32::from_le_bytes([data[offset+12], data[offset+13], data[offset+14], data[offset+15]]);
                    let dy = f32::from_le_bytes([data[offset+16], data[offset+17], data[offset+18], data[offset+19]]);
                    offset += 20;
                    PAN_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) { f(GestureEvent::pan(node_id, x, y, dx, dy, GesturePhase::Changed)); }
                    });
                }
            }
            OpCode::DirectGesturePanEnd => {
                if offset + 12 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let x = f32::from_le_bytes([data[offset+4], data[offset+5], data[offset+6], data[offset+7]]);
                    let y = f32::from_le_bytes([data[offset+8], data[offset+9], data[offset+10], data[offset+11]]);
                    offset += 12;
                    PAN_HANDLERS.with(|h| {
                        if let Some(f) = h.borrow_mut().get_mut(&node_id) {
                            f(GestureEvent::pan(node_id, x, y, 0.0, 0.0, GesturePhase::Ended));
                        }
                    });
                }
            }
            // === Unified V2 Gesture Events (replaces all legacy/direct events) ===
            OpCode::GestureEventV2 => {
                if offset + 14 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let event_type = data[offset+4];
                    let phase = data[offset+5];
                    let x = f32::from_le_bytes([data[offset+6], data[offset+7], data[offset+8], data[offset+9]]);
                    let y = f32::from_le_bytes([data[offset+10], data[offset+11], data[offset+12], data[offset+13]]);
                    offset += 14;
                    dispatch_v2_gesture_event(node_id, event_type, phase, x, y, 0);
                }
            }
            OpCode::GestureEventV2Ex => {
                if offset + 18 <= data.len() {
                    let node_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
                    let event_type = data[offset+4];
                    let phase = data[offset+5];
                    let x = f32::from_le_bytes([data[offset+6], data[offset+7], data[offset+8], data[offset+9]]);
                    let y = f32::from_le_bytes([data[offset+10], data[offset+11], data[offset+12], data[offset+13]]);
                    let payload = u32::from_le_bytes([data[offset+14], data[offset+15], data[offset+16], data[offset+17]]);
                    offset += 18;
                    dispatch_v2_gesture_event(node_id, event_type, phase, x, y, payload);
                }
            }
            _ => { offset += op.data_len(); }
        }
    }
}

// Legacy tap dispatch with bubbling - kept for backward compatibility with GestureTap commands
// New code uses DirectGestureTap which doesn't require bubbling
/// V2 Gesture Event Type constants (must match bridge.rs)
const V2_GESTURE_TYPE_TAP: u8 = 0;
const V2_GESTURE_TYPE_LONG_PRESS: u8 = 1;
const V2_GESTURE_TYPE_PAN: u8 = 2;
const V2_GESTURE_TYPE_SCALE: u8 = 3;
const V2_GESTURE_TYPE_ROTATION: u8 = 4;

const V2_GESTURE_PHASE_BEGAN: u8 = 0;
const V2_GESTURE_PHASE_CHANGED: u8 = 1;
const V2_GESTURE_PHASE_ENDED: u8 = 2;
const V2_GESTURE_PHASE_CANCELLED: u8 = 3;

/// Decode u32 payload to delta values
fn decode_v2_payload_dxdy(payload: u32) -> (f32, f32) {
    let dx = ((payload >> 16) as i16) as f32 / 100.0;
    let dy = (payload as i16) as f32 / 100.0;
    (dx, dy)
}

/// Decode u32 payload to scale value
fn decode_v2_payload_scale(payload: u32) -> f32 {
    let int_part = ((payload >> 24) & 0xFF) as f32;
    let frac_part = ((payload >> 16) & 0xFF) as f32 / 256.0;
    int_part + frac_part
}

/// Dispatch unified V2 gesture event to appropriate handler
fn dispatch_v2_gesture_event(node_id: u32, event_type: u8, phase: u8, x: f32, y: f32, payload: u32) {
    use crate::gesture::GesturePhase;

    let gesture_phase = match phase {
        V2_GESTURE_PHASE_BEGAN => GesturePhase::Began,
        V2_GESTURE_PHASE_CHANGED => GesturePhase::Changed,
        V2_GESTURE_PHASE_ENDED => GesturePhase::Ended,
        V2_GESTURE_PHASE_CANCELLED => GesturePhase::Cancelled,
        _ => GesturePhase::Ended,
    };

    match event_type {
        V2_GESTURE_TYPE_TAP => {
            let tap_count = if payload == 0 { 1 } else { payload };
            // All tap events (single, double, triple+) go through TAP_HANDLERS
            // Dispatch to appropriate handler based on tap_count
            TAP_HANDLERS.with(|h| {
                if let Some(entry) = h.borrow_mut().get_mut(&node_id) {
                    entry.dispatch(GestureEvent::tap(node_id, x, y, tap_count));
                }
            });
        }
        V2_GESTURE_TYPE_LONG_PRESS => {
            LONG_PRESS_HANDLERS.with(|h| {
                if let Some(f) = h.borrow_mut().get_mut(&node_id) {
                    f(GestureEvent::long_press(node_id, x, y, gesture_phase));
                }
            });
        }
        V2_GESTURE_TYPE_PAN => {
            let (dx, dy) = if phase == V2_GESTURE_PHASE_CHANGED {
                decode_v2_payload_dxdy(payload)
            } else {
                (0.0, 0.0)
            };
            PAN_HANDLERS.with(|h| {
                if let Some(f) = h.borrow_mut().get_mut(&node_id) {
                    f(GestureEvent::pan(node_id, x, y, dx, dy, gesture_phase));
                }
            });
        }
        V2_GESTURE_TYPE_SCALE => {
            let scale = if phase == V2_GESTURE_PHASE_CHANGED {
                decode_v2_payload_scale(payload)
            } else {
                1.0
            };
            SCALE_HANDLERS.with(|h| {
                if let Some(f) = h.borrow_mut().get_mut(&node_id) {
                    f(GestureEvent::scale(node_id, x, y, scale, 0.0, gesture_phase));
                }
            });
        }
        V2_GESTURE_TYPE_ROTATION => {
            ROTATION_HANDLERS.with(|h| {
                if let Some(f) = h.borrow_mut().get_mut(&node_id) {
                    f(GestureEvent::rotation(node_id, x, y, 0.0, 0.0, gesture_phase));
                }
            });
        }
        _ => {}
    }
}

fn dispatch_tap_with_bubble(node_id: u32, x: f32, y: f32) {
    // Simple direct dispatch - no bubbling since Host now handles it via HandlerRegistry
    TAP_HANDLERS.with(|h| {
        if let Some(entry) = h.borrow_mut().get_mut(&node_id) {
            entry.dispatch(GestureEvent::tap(node_id, x, y, 1));
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn dyxel_view_tick() {
    process_gesture_commands();
    
    unsafe { 
        LAST_SELECTED_NODE = None; 
        for i in 0..32 { SHARED_BUFFER.dirty_mask[i] = 0; }
    }
    
    EXECUTOR.with(|ex| {
        let mut tasks = ex.borrow_mut();
        let mut i = 0;
        while i < tasks.len() {
            let waker = futures_util::task::noop_waker();
            let mut cx = std::task::Context::from_waker(&waker);
            if tasks[i].as_mut().poll(&mut cx).is_ready() { let _ = tasks.remove(i); } else { i += 1; }
        }
    });
    
    // Note: PENDING_CLICKS removed - handlers are now called directly during process_gesture_commands
}

#[unsafe(no_mangle)]
pub extern "C" fn on_node_click(id: u32) {
    CLICK_COUNT.fetch_add(1, Ordering::SeqCst);
    CLICK_HANDLERS.with(|h| { if let Some(f) = h.borrow_mut().get_mut(&id) { f(); } });
    dyxel_view_tick();
}

#[unsafe(no_mangle)]
pub extern "C" fn dyxel_get_click_count() -> u32 { CLICK_COUNT.load(Ordering::SeqCst) }
#[unsafe(no_mangle)]
pub extern "C" fn dyxel_get_event_count() -> u32 { EVENT_COUNT.load(Ordering::SeqCst) }
#[unsafe(no_mangle)]
pub extern "C" fn dyxel_get_gesture_count() -> u32 { GESTURE_COUNT.load(Ordering::SeqCst) }

// ===== Property System =====

pub enum Prop<T> { Static(T), Dynamic(Box<dyn Signal<Item = T> + Unpin + 'static>) }

impl From<Dimension> for Prop<Dimension> { fn from(v: Dimension) -> Self { Prop::Static(v) } }
impl From<FlexDirection> for Prop<FlexDirection> { fn from(v: FlexDirection) -> Self { Prop::Static(v) } }
impl From<JustifyContent> for Prop<JustifyContent> { fn from(v: JustifyContent) -> Self { Prop::Static(v) } }
impl From<AlignItems> for Prop<AlignItems> { fn from(v: AlignItems) -> Self { Prop::Static(v) } }
impl From<FlexWrap> for Prop<FlexWrap> { fn from(v: FlexWrap) -> Self { Prop::Static(v) } }
impl From<AlignContent> for Prop<AlignContent> { fn from(v: AlignContent) -> Self { Prop::Static(v) } }
impl From<&str> for Prop<Dimension> { fn from(v: &str) -> Self { Prop::Static(Dimension::from(v)) } }
impl From<f32> for Prop<Dimension> { fn from(v: f32) -> Self { Prop::Static(Dimension::Pixels(v)) } }
impl From<String> for Prop<String> { fn from(v: String) -> Self { Prop::Static(v) } }
impl From<&str> for Prop<String> { fn from(v: &str) -> Self { Prop::Static(v.to_string()) } }
impl From<&String> for Prop<String> { fn from(v: &String) -> Self { Prop::Static(v.clone()) } }
impl From<f32> for Prop<f32> { fn from(v: f32) -> Self { Prop::Static(v) } }
impl From<i32> for Prop<i32> { fn from(v: i32) -> Self { Prop::Static(v) } }
impl From<u16> for Prop<u16> { fn from(v: u16) -> Self { Prop::Static(v) } }
impl From<bool> for Prop<bool> { fn from(v: bool) -> Self { Prop::Static(v) } }
impl From<(u32,u32,u32)> for Prop<(u32,u32,u32)> { fn from(v: (u32,u32,u32)) -> Self { Prop::Static(v) } }
impl From<(u32,u32,u32,u32)> for Prop<(u32,u32,u32,u32)> { fn from(v: (u32,u32,u32,u32)) -> Self { Prop::Static(v) } }
impl From<(u8,u8,u8,u8)> for Prop<(u8,u8,u8,u8)> { fn from(v: (u8,u8,u8,u8)) -> Self { Prop::Static(v) } }
impl From<(f32,f32,f32,f32)> for Prop<(f32,f32,f32,f32)> { fn from(v: (f32,f32,f32,f32)) -> Self { Prop::Static(v) } }
impl From<(f32,f32,f32,u32)> for Prop<(f32,f32,f32,u32)> { fn from(v: (f32,f32,f32,u32)) -> Self { Prop::Static(v) } }
impl From<(f32,f32)> for Prop<(f32,f32)> { fn from(v: (f32,f32)) -> Self { Prop::Static(v) } }
// SizeUnit support
impl From<SizeUnit> for Prop<SizeUnit> { fn from(v: SizeUnit) -> Self { Prop::Static(v) } }
impl From<Dimension> for Prop<SizeUnit> { 
    fn from(v: Dimension) -> Self { 
        match v {
            Dimension::Auto => Prop::Static(SizeUnit::Auto),
            Dimension::Pixels(px) => Prop::Static(SizeUnit::Px(px)),
            Dimension::Percent(pct) => Prop::Static(SizeUnit::Percent(pct)),
        }
    } 
}
impl From<f32> for Prop<SizeUnit> { fn from(v: f32) -> Self { Prop::Static(SizeUnit::Lp(v)) } }
impl From<i32> for Prop<SizeUnit> { fn from(v: i32) -> Self { Prop::Static(SizeUnit::Lp(v as f32)) } }
impl From<&str> for Prop<SizeUnit> { fn from(v: &str) -> Self { Prop::Static(SizeUnit::from(v)) } }

// Support for State signals
impl From<dyxel_state::SizeUnitSignal> for Prop<SizeUnit> { 
    fn from(v: dyxel_state::SizeUnitSignal) -> Self { Prop::Dynamic(Box::new(v)) } 
}
impl From<dyxel_state::StateSignal<(u32, u32, u32, u32)>> for Prop<(u32, u32, u32, u32)> { 
    fn from(v: dyxel_state::StateSignal<(u32, u32, u32, u32)>) -> Self { Prop::Dynamic(Box::new(v)) } 
}

pub trait SignalPropExt: Signal + Sized { 
    fn sig(self) -> Prop<Self::Item> where Self: Unpin + 'static { Prop::Dynamic(Box::new(self)) }
    
    /// Convert f32 signal to SizeUnit signal (for width/height binding)
    fn sig_size(self) -> Prop<SizeUnit> where Self: Signal<Item = f32> + Unpin + 'static {
        Prop::Dynamic(Box::new(self.map(SizeUnit::Lp)))
    }
    
    /// Convert (u32,u32,u32,u32) signal to color Prop (for color binding)
    fn sig_color(self) -> Prop<(u32,u32,u32,u32)> where Self: Signal<Item = (u32,u32,u32,u32)> + Unpin + 'static {
        Prop::Dynamic(Box::new(self))
    }
}
impl<S: Signal + SignalExt> SignalPropExt for S {}

static NODE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn apply_prop<T: 'static, F>(id: u32, p: Prop<T>, f: F) where F: Fn(u32, T) + 'static {
    match p {
        Prop::Static(v) => f(id, v),
        Prop::Dynamic(s) => {
            let future = s.for_each(move |val| { f(id, val); async {} });
            EXECUTOR.with(|ex| ex.borrow_mut().push(Box::pin(future)));
        }
    }
}

// ===== BaseView Trait =====

pub trait BaseView {
    fn node_id(&self) -> u32;
    
    fn color(self, p: impl Into<Prop<(u32,u32,u32,u32)>>) -> Self where Self: Sized {
        apply_prop(self.node_id(), p.into(), |id, (r, g, b, a)| {
            select_node(id);
            push_command!(SHARED_BUFFER, SetColorCompact, r as u8, g as u8, b as u8, a as u8);
        }); self 
    }
    
    fn width(self, p: impl Into<Prop<SizeUnit>>) -> Self where Self: Sized {
        apply_prop(self.node_id(), p.into(), |id, unit| {
            select_node(id);
            unsafe {
                let device_info = &*(&raw const SHARED_BUFFER.device_info);
                let (t, v) = match unit {
                    SizeUnit::Auto => (0u8, 0.0f32),
                    SizeUnit::Lp(lp) => (1, device_info.lp_to_px(lp)), // Convert LP to PX for layout
                    SizeUnit::Px(px) => (1, px),
                    SizeUnit::Percent(pct) => (2, pct),
                };
                push_command!(SHARED_BUFFER, SetWidthCompact, t, v);
            }
        }); self 
    }
    
    fn height(self, p: impl Into<Prop<SizeUnit>>) -> Self where Self: Sized {
        apply_prop(self.node_id(), p.into(), |id, unit| {
            select_node(id);
            unsafe {
                let device_info = &*(&raw const SHARED_BUFFER.device_info);
                let (t, v) = match unit {
                    SizeUnit::Auto => (0u8, 0.0f32),
                    SizeUnit::Lp(lp) => (1, device_info.lp_to_px(lp)), // Convert LP to PX for layout
                    SizeUnit::Px(px) => (1, px),
                    SizeUnit::Percent(pct) => (2, pct),
                };
                push_command!(SHARED_BUFFER, SetHeightCompact, t, v);
            }
        }); self 
    }
    
    fn flex_direction(self, p: impl Into<Prop<FlexDirection>>) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, dir| { select_node(id); push_command!(SHARED_BUFFER, SetFlexDirection, id, dir as u32); }); self 
    }
    fn justify_content(self, p: impl Into<Prop<JustifyContent>>) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, j| { select_node(id); push_command!(SHARED_BUFFER, SetJustifyContent, id, j as u32); }); self 
    }
    fn align_items(self, p: impl Into<Prop<AlignItems>>) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, a| { select_node(id); push_command!(SHARED_BUFFER, SetAlignItems, id, a as u32); }); self 
    }
    fn flex_wrap(self, p: impl Into<Prop<FlexWrap>>) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, w| { select_node(id); push_command!(SHARED_BUFFER, SetFlexWrap, id, w as u32); }); self 
    }
    fn align_content(self, p: impl Into<Prop<AlignContent>>) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, ac| { select_node(id); push_command!(SHARED_BUFFER, SetAlignContent, id, ac as u32); }); self 
    }
    fn flex_grow(self, p: impl Into<Prop<f32>>) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, grow| { select_node(id); push_command!(SHARED_BUFFER, SetFlexGrow, id, grow); }); self 
    }
    fn z_index(self, p: impl Into<Prop<i32>>) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, z| { select_node(id); push_command!(SHARED_BUFFER, SetZIndex, id, z); }); self 
    }
    fn padding(self, p: impl Into<Prop<(f32,f32,f32,f32)>>) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, (t, r, b, l)| { 
            select_node(id); 
            push_command!(SHARED_BUFFER, SetPadding, id, t, r, b, l); 
        }); self 
    }
    fn margin(self, p: impl Into<Prop<(f32,f32,f32,f32)>>) -> Self where Self: Sized {
        // Note: margin not implemented yet, uses padding as placeholder
        self.padding(p)
    }
    fn border_radius(self, p: impl Into<Prop<f32>>) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, r| { select_node(id); push_command!(SHARED_BUFFER, SetBorderRadius, id, r); }); self 
    }
    fn on_click(self, handler: impl FnMut() + 'static) -> Self where Self: Sized {
        let id = self.node_id(); 
        select_node(id); 
        push_command!(SHARED_BUFFER, AttachClick, id); 
        CLICK_HANDLERS.with(|h| { h.borrow_mut().insert(id, Box::new(handler)); }); 
        self
    }
    /// On tap handler - receives GestureEvent
    fn on_tap(self, handler: impl FnMut(GestureEvent) + 'static) -> Self where Self: Sized {
        let id = self.node_id();
        select_node(id);
        push_command!(SHARED_BUFFER, AttachClick, id);
        push_command!(SHARED_BUFFER, RegisterTapHandler, id, 1u32); // Notify Host (count=1 for single tap)
        // Store in the appropriate slot of TapHandlerEntry
        TAP_HANDLERS.with(|h| {
            let mut handlers = h.borrow_mut();
            let entry = handlers.entry(id).or_insert_with(TapHandlerEntry::new);
            entry.single_tap = Some(Box::new(handler));
        });
        self
    }
    /// On double tap handler - receives GestureEvent
    /// Internally uses RegisterTapHandler with count=2 (unified tap command)
    fn on_double_tap(self, handler: impl FnMut(GestureEvent) + 'static) -> Self where Self: Sized {
        let id = self.node_id();
        select_node(id);
        push_command!(SHARED_BUFFER, AttachClick, id);
        // Use unified RegisterTapHandler with count=2
        push_command!(SHARED_BUFFER, RegisterTapHandler, id, 2u32);
        // Store in the appropriate slot of TapHandlerEntry
        TAP_HANDLERS.with(|h| {
            let mut handlers = h.borrow_mut();
            let entry = handlers.entry(id).or_insert_with(TapHandlerEntry::new);
            entry.double_tap = Some(Box::new(handler));
        });
        self
    }
    /// On long press handler - receives GestureEvent
    /// Use `event.phase` to check state (Began/Ended/Cancelled)
    fn on_long_press(self, handler: impl FnMut(GestureEvent) + 'static) -> Self where Self: Sized {
        let id = self.node_id();
        select_node(id);
        push_command!(SHARED_BUFFER, AttachClick, id);
        push_command!(SHARED_BUFFER, RegisterLongPressHandler, id); // Notify Host
        LONG_PRESS_HANDLERS.with(|h| {
            h.borrow_mut().insert(id, Box::new(handler));
        });
        self
    }
    /// On pan handler - receives GestureEvent
    /// Use `event.phase` to check state (Began/Changed/Ended/Cancelled)
    fn on_pan(self, handler: impl FnMut(GestureEvent) + 'static) -> Self where Self: Sized {
        let id = self.node_id();
        select_node(id);
        push_command!(SHARED_BUFFER, AttachClick, id);
        push_command!(SHARED_BUFFER, RegisterPanHandler, id); // Notify Host
        PAN_HANDLERS.with(|h| {
            h.borrow_mut().insert(id, Box::new(handler));
        });
        self
    }
    /// On scale handler - receives GestureEvent
    /// Use `event.phase` to check state (Began/Changed/Ended/Cancelled)
    fn on_scale(self, handler: impl FnMut(GestureEvent) + 'static) -> Self where Self: Sized {
        let id = self.node_id();
        select_node(id);
        push_command!(SHARED_BUFFER, AttachClick, id);
        push_command!(SHARED_BUFFER, RegisterScaleHandler, id); // Notify Host
        SCALE_HANDLERS.with(|h| {
            h.borrow_mut().insert(id, Box::new(handler));
        });
        self
    }
    /// On rotation handler - receives GestureEvent
    /// Use `event.phase` to check state (Began/Changed/Ended/Cancelled)
    fn on_rotation(self, handler: impl FnMut(GestureEvent) + 'static) -> Self where Self: Sized {
        let id = self.node_id();
        select_node(id);
        push_command!(SHARED_BUFFER, AttachClick, id);
        push_command!(SHARED_BUFFER, RegisterRotationHandler, id); // Notify Host
        ROTATION_HANDLERS.with(|h| {
            h.borrow_mut().insert(id, Box::new(handler));
        });
        self
    }

    // === Layer Effects (Vello Native) ===
    fn opacity(self, p: impl Into<Prop<f32>>) -> Self where Self: Sized {
        apply_prop(self.node_id(), p.into(), |id, opacity| {
            select_node(id);
            push_command!(SHARED_BUFFER, SetOpacity, id, opacity.clamp(0.0, 1.0));
        }); self
    }
    fn shadow_offset(self, x: f32, y: f32) -> Self where Self: Sized {
        let id = self.node_id();
        select_node(id);
        // Store partial shadow config - full shadow set via shadow() method
        SHADOW_CONFIG.with(|c| {
            let mut config = c.borrow_mut();
            config.entry(id).or_insert_with(ShadowConfig::default).offset_x = x;
            config.entry(id).or_insert_with(ShadowConfig::default).offset_y = y;
        });
        self
    }
    fn shadow_blur(self, blur: f32) -> Self where Self: Sized {
        let id = self.node_id();
        select_node(id);
        SHADOW_CONFIG.with(|c| {
            let mut config = c.borrow_mut();
            config.entry(id).or_insert_with(ShadowConfig::default).blur = blur;
        });
        self
    }
    fn shadow_color(self, color: impl Into<Prop<u32>>) -> Self where Self: Sized {
        let id = self.node_id();
        select_node(id);
        apply_prop(id, color.into(), |node_id, color_val| {
            SHADOW_CONFIG.with(|c| {
                let mut config = c.borrow_mut();
                config.entry(node_id).or_insert_with(ShadowConfig::default).color = color_val;
            });
        });
        self
    }
    fn shadow(self, params: impl Into<Prop<(f32, f32, f32, u32)>>) -> Self where Self: Sized {
        let id = self.node_id();
        apply_prop(id, params.into(), |node_id, (offset_x, offset_y, blur, color)| {
            select_node(node_id);
            push_command!(SHARED_BUFFER, SetShadow, node_id, offset_x, offset_y, blur, color);
        });
        self
    }
    fn blur(self, p: impl Into<Prop<f32>>) -> Self where Self: Sized {
        apply_prop(self.node_id(), p.into(), |id, radius| {
            select_node(id);
            push_command!(SHARED_BUFFER, SetBlur, id, radius.max(0.0));
        }); self
    }
    fn clip_to_bounds(self, clip: impl Into<Prop<bool>>) -> Self where Self: Sized {
        let id = self.node_id();
        apply_prop(id, clip.into(), |node_id, clip_val| {
            select_node(node_id);
            push_command!(SHARED_BUFFER, SetClipToBounds, node_id, if clip_val { 1u8 } else { 0u8 });
        });
        self
    }
    fn position(self, pos: impl Into<Prop<(f32, f32)>>) -> Self where Self: Sized {
        let id = self.node_id();
        apply_prop(id, pos.into(), |node_id, (x, y)| {
            select_node(node_id);
            push_command!(SHARED_BUFFER, SetPosition, node_id, x, y);
        });
        self
    }
    fn child(self, child_id: u32) -> Self where Self: Sized { 
        let parent_id = self.node_id();
        select_node(parent_id); 
        push_command!(SHARED_BUFFER, AddChild, parent_id, child_id);
        // Note: PARENT_MAP removed - Host now handles event bubbling via HandlerRegistry
        self 
    }
}

// ===== View Components =====

pub struct View { pub id: u32 }
impl View {
    pub fn new() -> Self {
        let id = NODE_COUNTER.fetch_add(1, Ordering::SeqCst); 
        track_node(id);
        push_command!(SHARED_BUFFER, CreateNode, id);
        unsafe { LAST_SELECTED_NODE = Some(id); } 
        Self { id }
            .width(Dimension::Auto)
            .height(Dimension::Auto)
            .flex_direction(FlexDirection::Row)
            .justify_content(JustifyContent::FlexStart)
            .align_items(AlignItems::FlexStart)
            .flex_wrap(FlexWrap::Wrap)
    }
    
    /// Simple tap handler without coordinates (for convenience)
    pub fn on_tap_simple(self, mut handler: impl FnMut() + 'static) -> Self {
        let id = self.node_id();
        select_node(id);
        push_command!(SHARED_BUFFER, AttachClick, id);
        push_command!(SHARED_BUFFER, RegisterTapHandler, id, 1u32); // Notify Host (count=1 for single tap)
        TAP_HANDLERS.with(|h| {
            let mut handlers = h.borrow_mut();
            let entry = handlers.entry(id).or_insert_with(TapHandlerEntry::new);
            entry.single_tap = Some(Box::new(move |_| handler()));
        });
        self
    }
    
    /// Configure a composite gesture (Sequence, Parallel, or Exclusive)
    /// 
    /// Example:
    /// ```rust
    /// View::new().gesture(
    ///     SequenceGesture::new(vec![
    ///         TapGesture::double_tap().into(),
    ///         LongPressGesture::new().into(),
    ///     ]).on_gesture_judge_begin(|e| true)
    /// )
    /// ```
    pub fn gesture(self, config: impl GestureConfig) -> Self {
        config.apply_to(self)
    }
}
impl BaseView for View { fn node_id(&self) -> u32 { self.id } }

/// Trait for gesture configurations that can be applied to a View
pub trait GestureConfig {
    fn apply_to(self, view: View) -> View;
}

impl GestureConfig for crate::SequenceGesture {
    fn apply_to(self, view: View) -> View {
        let id = view.node_id();
        crate::select_node(id);

        // Register all steps in sequence
        for step in &self.steps {
            match step {
                crate::GestureStep::Tap(g) => {
                    push_tap_registration(id, g.count);
                }
                crate::GestureStep::LongPress(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterLongPressHandler, id);
                }
                crate::GestureStep::Pan(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterPanHandler, id);
                }
                crate::GestureStep::Scale(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterScaleHandler, id);
                }
                crate::GestureStep::Rotation(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterRotationHandler, id);
                }
                // Nested gestures - recursively register their components
                crate::GestureStep::Exclusive(exclusive) => {
                    Self::register_exclusive_gesture(id, exclusive);
                }
                crate::GestureStep::Simultaneous(simultaneous) => {
                    Self::register_parallel_gesture(id, simultaneous);
                }
                crate::GestureStep::Sequenced(sequenced) => {
                    Self::register_sequence_gesture(id, sequenced);
                }
            }
        }

        // Store sequence configuration for runtime processing
        view
    }
}

impl crate::SequenceGesture {
    fn register_exclusive_gesture(node_id: u32, gesture: &crate::ExclusiveGesture) {
        for candidate in &gesture.candidates {
            match candidate {
                crate::GestureStep::Tap(g) => {
                    push_tap_registration(node_id, g.count);
                }
                crate::GestureStep::LongPress(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterLongPressHandler, node_id);
                }
                crate::GestureStep::Pan(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterPanHandler, node_id);
                }
                crate::GestureStep::Scale(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterScaleHandler, node_id);
                }
                crate::GestureStep::Rotation(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterRotationHandler, node_id);
                }
                _ => {}
            }
        }
    }

    fn register_parallel_gesture(node_id: u32, gesture: &crate::ParallelGesture) {
        for gesture_step in &gesture.gestures {
            match gesture_step {
                crate::GestureStep::Tap(g) => {
                    push_tap_registration(node_id, g.count);
                }
                crate::GestureStep::LongPress(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterLongPressHandler, node_id);
                }
                crate::GestureStep::Pan(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterPanHandler, node_id);
                }
                crate::GestureStep::Scale(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterScaleHandler, node_id);
                }
                crate::GestureStep::Rotation(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterRotationHandler, node_id);
                }
                _ => {}
            }
        }
    }

    fn register_sequence_gesture(node_id: u32, gesture: &crate::SequenceGesture) {
        for step in &gesture.steps {
            match step {
                crate::GestureStep::Tap(g) => {
                    push_tap_registration(node_id, g.count);
                }
                crate::GestureStep::LongPress(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterLongPressHandler, node_id);
                }
                crate::GestureStep::Pan(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterPanHandler, node_id);
                }
                crate::GestureStep::Scale(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterScaleHandler, node_id);
                }
                crate::GestureStep::Rotation(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, node_id);
                    push_command!(SHARED_BUFFER, RegisterRotationHandler, node_id);
                }
                _ => {}
            }
        }
    }
}

impl GestureConfig for crate::ParallelGesture {
    fn apply_to(self, view: View) -> View {
        let id = view.node_id();
        crate::select_node(id);

        // Register all gestures to be recognized simultaneously
        for gesture in &self.gestures {
            match gesture {
                crate::GestureStep::Tap(g) => {
                    push_tap_registration(id, g.count);
                }
                crate::GestureStep::LongPress(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterLongPressHandler, id);
                }
                crate::GestureStep::Pan(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterPanHandler, id);
                }
                crate::GestureStep::Scale(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterScaleHandler, id);
                }
                crate::GestureStep::Rotation(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterRotationHandler, id);
                }
                // Nested gestures
                crate::GestureStep::Exclusive(exclusive) => {
                    crate::SequenceGesture::register_exclusive_gesture(id, exclusive);
                }
                crate::GestureStep::Simultaneous(simultaneous) => {
                    crate::SequenceGesture::register_parallel_gesture(id, simultaneous);
                }
                crate::GestureStep::Sequenced(sequenced) => {
                    crate::SequenceGesture::register_sequence_gesture(id, sequenced);
                }
            }
        }

        view
    }
}

impl GestureConfig for crate::ExclusiveGesture {
    fn apply_to(self, view: View) -> View {
        let id = view.node_id();
        crate::select_node(id);

        // Register all candidates - GestureArena will handle competition
        for candidate in &self.candidates {
            match candidate {
                crate::GestureStep::Tap(g) => {
                    push_tap_registration(id, g.count);
                }
                crate::GestureStep::LongPress(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterLongPressHandler, id);
                }
                crate::GestureStep::Pan(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterPanHandler, id);
                }
                crate::GestureStep::Scale(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterScaleHandler, id);
                }
                crate::GestureStep::Rotation(_) => {
                    push_command!(SHARED_BUFFER, AttachClick, id);
                    push_command!(SHARED_BUFFER, RegisterRotationHandler, id);
                }
                // Nested gestures
                crate::GestureStep::Exclusive(exclusive) => {
                    crate::SequenceGesture::register_exclusive_gesture(id, exclusive);
                }
                crate::GestureStep::Simultaneous(simultaneous) => {
                    crate::SequenceGesture::register_parallel_gesture(id, simultaneous);
                }
                crate::GestureStep::Sequenced(sequenced) => {
                    crate::SequenceGesture::register_sequence_gesture(id, sequenced);
                }
            }
        }

        view
    }
}

/// Push tap registration commands for a specific count
/// All tap gestures (single, double, triple, etc.) use the same command with count parameter
fn push_tap_registration(node_id: u32, count: u32) {
    crate::select_node(node_id);
    push_command!(SHARED_BUFFER, AttachClick, node_id);
    push_command!(SHARED_BUFFER, RegisterTapHandler, node_id, count);
}

impl GestureConfig for crate::TapGesture {
    fn apply_to(self, view: View) -> View {
        let id = view.node_id();
        let count = self.count;
        push_tap_registration(id, count);

        // Store the callback if provided
        if let Some(mut on_ended) = self.on_gesture_ended {
            TAP_HANDLERS.with(|h| {
                let mut handlers = h.borrow_mut();
                let entry = handlers.entry(id).or_insert_with(TapHandlerEntry::new);
                // Store in appropriate slot based on count
                match count {
                    1 => entry.single_tap = Some(Box::new(move |e| {
                        if e.tap_count == count { on_ended(e); }
                    })),
                    2 => entry.double_tap = Some(Box::new(move |e| {
                        if e.tap_count == count { on_ended(e); }
                    })),
                    _ => entry.multi_tap = Some(Box::new(move |e| {
                        if e.tap_count == count { on_ended(e); }
                    })),
                }
            });
        }

        view
    }
}

pub struct Text { pub id: u32 }
impl Text {
    pub fn new() -> Self {
        let id = NODE_COUNTER.fetch_add(1, Ordering::SeqCst); 
        track_node(id);
        unsafe {
            push_command!(SHARED_BUFFER, CreateTextNode, id);
            LAST_SELECTED_NODE = Some(id);
        }
        Self { id }
    }
    pub fn value(self, p: impl Into<Prop<String>>) -> Self { 
        apply_prop(self.id, p.into(), |id, s| {
            select_node(id);
            let len = s.len() as u32;
            unsafe {
                push_command!(SHARED_BUFFER, SetTextContent, id, len);
                let offset = SHARED_BUFFER.command_len as usize;
                if offset + s.len() <= MAX_COMMAND_BYTES {
                    SHARED_BUFFER.command_data[offset..offset+s.len()].copy_from_slice(s.as_bytes());
                    SHARED_BUFFER.command_len = (offset + s.len()) as u32;
                }
            }
        }); self 
    }
    pub fn font_size(self, p: impl Into<Prop<f32>>) -> Self { 
        apply_prop(self.id, p.into(), |id, size| { select_node(id); push_command!(SHARED_BUFFER, SetFontSize, id, size); }); self 
    }
    pub fn font_weight(self, p: impl Into<Prop<u16>>) -> Self { 
        apply_prop(self.id, p.into(), |id, weight| { select_node(id); push_command!(SHARED_BUFFER, SetTextWeight, id, weight); }); self 
    }
    pub fn text_color(self, p: impl Into<Prop<(u8,u8,u8,u8)>>) -> Self { 
        apply_prop(self.id, p.into(), |id, (r,g,b,a)| { select_node(id); push_command!(SHARED_BUFFER, SetTextColor, id, r, g, b, a); }); self 
    }
}
impl BaseView for Text { fn node_id(&self) -> u32 { self.id } }

/// Column - vertical flex container (RSX-friendly)
pub struct Column { view: View }
impl Column {
    pub fn new() -> Self {
        let view = View::new()
            .flex_direction(FlexDirection::Column);
        Self { view }
    }
    pub fn spacing(self, _value: f32) -> Self {
        // Note: spacing not directly supported, would need gap property
        self
    }
}
impl BaseView for Column {
    fn node_id(&self) -> u32 { self.view.node_id() }
}

/// Row - horizontal flex container (RSX-friendly)
pub struct Row { view: View }
impl Row {
    pub fn new() -> Self {
        let view = View::new()
            .flex_direction(FlexDirection::Row);
        Self { view }
    }
    pub fn spacing(self, _value: f32) -> Self {
        self
    }
}
impl BaseView for Row {
    fn node_id(&self) -> u32 { self.view.node_id() }
}

/// Button component (RSX-friendly)
pub struct Button { view: View }
impl Button {
    pub fn new() -> Self {
        let view = View::new()
            .color((60u32, 120, 220, 255))
            .padding((10.0, 20.0, 10.0, 20.0))
            .border_radius(8.0);
        Self { view }
    }
    pub fn on_tap(self, handler: impl FnMut(GestureEvent) + 'static) -> Self {
        let id = self.node_id();
        select_node(id);
        push_command!(SHARED_BUFFER, AttachClick, id);
        push_command!(SHARED_BUFFER, RegisterTapHandler, id, 1u32); // Notify Host (count=1 for single tap)
        TAP_HANDLERS.with(|h| {
            let mut handlers = h.borrow_mut();
            let entry = handlers.entry(id).or_insert_with(TapHandlerEntry::new);
            entry.single_tap = Some(Box::new(handler));
        });
        self
    }
}
impl BaseView for Button {
    fn node_id(&self) -> u32 { self.view.node_id() }
}

// ===== Utilities =====

pub fn force_layout() { 
    push_command!(SHARED_BUFFER, UpdateLayout); 
    unsafe { ui_force_layout(); } 
}

pub fn set_text(id: u32, text: &str) {
    select_node(id);
    let len = text.len() as u32;
    push_command!(SHARED_BUFFER, SetTextContent, id, len);
    unsafe {
        let offset = SHARED_BUFFER.command_len as usize;
        let bytes = text.as_bytes();
        if offset + bytes.len() <= dyxel_shared::MAX_COMMAND_BYTES {
            SHARED_BUFFER.command_data[offset..offset+bytes.len()].copy_from_slice(bytes);
            SHARED_BUFFER.command_len = (offset + bytes.len()) as u32;
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
extern "C" { 
    fn ui_force_layout();
    fn console_log(ptr: *const u8, len: usize);
}

// Stubs for non-WASM targets
#[cfg(not(target_arch = "wasm32"))]
unsafe fn ui_force_layout() {}
#[cfg(not(target_arch = "wasm32"))]
unsafe fn console_log(_ptr: *const u8, _len: usize) {}

/// Log message to host console
pub fn log(msg: &str) {
    unsafe {
        console_log(msg.as_ptr(), msg.len());
    }
}

/// Spawn an async task
pub fn spawn(task: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
    EXECUTOR.with(|ex| {
        ex.borrow_mut().push(task);
    });
}
