// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use futures_signals::signal::{Signal, SignalExt};
use std::sync::atomic::{AtomicU32, Ordering};
use std::cell::RefCell;
use std::collections::HashMap;
pub use dyxel_shared::{FlexDirection, JustifyContent, AlignItems, FlexWrap, AlignContent, Dimension, Role, ViewType, OpCode, LayoutResult, MAX_COMMAND_BYTES, SharedBuffer, DirtyField, TransactionFlags};
use dyxel_shared::push_command;

// Dual-Track WASM API (Week 4)
pub mod dual_track_wasm;

// --- Command Stream ---
#[no_mangle]
pub static mut SHARED_BUFFER: SharedBuffer = SharedBuffer {
    command_len: 0,
    max_node_id: 0,
    _padding: [0; 2],
    command_data: [0; MAX_COMMAND_BYTES],
    layout_results: [LayoutResult { x: 0.0, y: 0.0, width: 0.0, height: 0.0 }; dyxel_shared::MAX_NODES],
    dirty_mask: [0; 32], 
};

#[no_mangle]
pub extern "C" fn dyxel_get_protocol_hash() -> u64 {
    dyxel_shared::PROTOCOL_HASH
}

#[no_mangle]
pub extern "C" fn dyxel_get_shared_buffer_ptr() -> u32 {
    std::ptr::addr_of!(SHARED_BUFFER) as u32
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
pub fn get_layout(id: u32) -> LayoutResult { unsafe { SHARED_BUFFER.layout_results[id as usize] } }

// === Transaction API (NEW!) ===

/// Transaction handle for batching commands
pub struct Transaction {
    seq_id: u32,
    start_offset: u32,
    committed: bool,
}

impl Transaction {
    /// Start a new transaction
    pub fn new(flags: u16) -> Self {
        unsafe {
            static mut NEXT_SEQ_ID: u32 = 1;
            let seq_id: u32 = NEXT_SEQ_ID;
            NEXT_SEQ_ID += 1;
            
            let start_offset: u32 = SHARED_BUFFER.command_len;
            
            // Emit BeginTransaction command
            push_command!(SHARED_BUFFER, BeginTransaction, seq_id, flags);
            
            Self {
                seq_id,
                start_offset,
                committed: false,
            }
        }
    }
    
    /// Commit the transaction
    pub fn commit(mut self) {
        self.committed = true;
        push_command!(SHARED_BUFFER, EndTransaction, self.seq_id);
    }
    
    /// Abort the transaction (commands are discarded)
    pub fn abort(mut self) {
        self.committed = true;
        unsafe {
            // Rollback command_len to before transaction started
            SHARED_BUFFER.command_len = self.start_offset;
        }
        push_command!(SHARED_BUFFER, AbortTransaction, self.seq_id);
    }
    
    /// Get the sequence ID of this transaction
    pub fn seq_id(&self) -> u32 {
        self.seq_id
    }
    
    /// Get the number of bytes written in this transaction so far
    pub fn size(&self) -> u32 {
        unsafe { SHARED_BUFFER.command_len - self.start_offset }
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.committed {
            // Auto-abort if not explicitly committed
            unsafe {
                SHARED_BUFFER.command_len = self.start_offset;
            }
            push_command!(SHARED_BUFFER, AbortTransaction, self.seq_id);
        }
    }
}

/// Convenience function: begin a transaction with default flags
pub fn begin_transaction() -> Transaction {
    Transaction::new(0)
}

/// Convenience function: begin a transaction with specific flags
pub fn begin_transaction_with_flags(flags: u16) -> Transaction {
    Transaction::new(flags)
}

/// Execute a closure within a transaction, auto-commit on success
pub fn with_transaction<F, R>(f: F) -> R
where
    F: FnOnce(&mut Transaction) -> R,
{
    let mut tx = begin_transaction();
    let result = f(&mut tx);
    tx.commit();
    result
}

/// Execute a closure within a transaction with flags, auto-commit on success
pub fn with_transaction_flags<F, R>(flags: u16, f: F) -> R
where
    F: FnOnce(&mut Transaction) -> R,
{
    let mut tx = Transaction::new(flags);
    let result = f(&mut tx);
    tx.commit();
    result
}

#[link(wasm_import_module = "env")]
extern "C" { fn ui_force_layout(); }

thread_local! {
    static EXECUTOR: RefCell<Vec<std::pin::Pin<Box<dyn futures_util::future::Future<Output = ()>>>>> = RefCell::new(Vec::new());
    static CLICK_HANDLERS: RefCell<HashMap<u32, Box<dyn FnMut()>>> = RefCell::new(HashMap::new());
}

#[no_mangle]
pub extern "C" fn dyxel_view_tick() {
    unsafe { 
        LAST_SELECTED_NODE = None; 
        
        // Reset dirty bitmap
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
}

#[no_mangle]
pub extern "C" fn on_node_click(id: u32) {
    CLICK_HANDLERS.with(|handlers| { if let Some(handler) = handlers.borrow_mut().get_mut(&id) { handler(); } });
    dyxel_view_tick();
}

pub enum Prop<T> { Static(T), Dynamic(Box<dyn Signal<Item = T> + Unpin + 'static>) }

// Specific implementations for common types to avoid overlapping blanket impls
impl From<Dimension> for Prop<Dimension> { fn from(v: Dimension) -> Self { Prop::Static(v) } }
impl From<FlexDirection> for Prop<FlexDirection> { fn from(v: FlexDirection) -> Self { Prop::Static(v) } }
impl From<JustifyContent> for Prop<JustifyContent> { fn from(v: JustifyContent) -> Self { Prop::Static(v) } }
impl From<AlignItems> for Prop<AlignItems> { fn from(v: AlignItems) -> Self { Prop::Static(v) } }
impl From<FlexWrap> for Prop<FlexWrap> { fn from(v: FlexWrap) -> Self { Prop::Static(v) } }
impl From<AlignContent> for Prop<AlignContent> { fn from(v: AlignContent) -> Self { Prop::Static(v) } }

impl From<f32> for Prop<f32> { fn from(v: f32) -> Self { Prop::Static(v) } }
impl From<i32> for Prop<i32> { fn from(v: i32) -> Self { Prop::Static(v) } }
impl From<u16> for Prop<u16> { fn from(v: u16) -> Self { Prop::Static(v) } }
impl From<(u32,u32,u32)> for Prop<(u32,u32,u32)> { fn from(v: (u32,u32,u32)) -> Self { Prop::Static(v) } }
impl From<(u8,u8,u8,u8)> for Prop<(u8,u8,u8,u8)> { fn from(v: (u8,u8,u8,u8)) -> Self { Prop::Static(v) } }
impl From<(f32,f32,f32,f32)> for Prop<(f32,f32,f32,f32)> { fn from(v: (f32,f32,f32,f32)) -> Self { Prop::Static(v) } }

impl From<&str> for Prop<Dimension> { fn from(v: &str) -> Self { Prop::Static(Dimension::from(v)) } }
impl From<f32> for Prop<Dimension> { fn from(v: f32) -> Self { Prop::Static(Dimension::from(v)) } }
impl From<i32> for Prop<Dimension> { fn from(v: i32) -> Self { Prop::Static(Dimension::from(v)) } }

// String conversions
impl From<String> for Prop<String> { fn from(v: String) -> Self { Prop::Static(v) } }
impl From<&str> for Prop<String> { fn from(v: &str) -> Self { Prop::Static(v.to_string()) } }
impl From<&String> for Prop<String> { fn from(v: &String) -> Self { Prop::Static(v.to_string()) } }

pub trait SignalPropExt: Signal + Sized { fn sig(self) -> Prop<Self::Item> where Self: Unpin + 'static { Prop::Dynamic(Box::new(self)) } }
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

pub trait BaseView {
    fn node_id(&self) -> u32;
    fn color<P: Into<Prop<(u32,u32,u32)>>>(self, p: P) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, (r, g, b)| {
            select_node(id);
            push_command!(SHARED_BUFFER, SetColorCompact, r as u8, g as u8, b as u8, 255u8);
        }); self 
    }
    fn width<P: Into<Prop<Dimension>>>(self, p: P) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, dim| {
            select_node(id);
            let (t, v) = match dim { Dimension::Auto => (0u8, 0.0f32), Dimension::Pixels(x) => (1, x), Dimension::Percent(x) => (2, x) };
            push_command!(SHARED_BUFFER, SetWidthCompact, t, v);
        }); self 
    }
    fn height<P: Into<Prop<Dimension>>>(self, p: P) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, dim| {
            select_node(id);
            let (t, v) = match dim { Dimension::Auto => (0u8, 0.0f32), Dimension::Pixels(x) => (1, x), Dimension::Percent(x) => (2, x) };
            push_command!(SHARED_BUFFER, SetHeightCompact, t, v);
        }); self 
    }
    fn flex_direction<P: Into<Prop<FlexDirection>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, dir| { select_node(id); push_command!(SHARED_BUFFER, SetFlexDirection, id, dir as u32); }); self }
    fn justify_content<P: Into<Prop<JustifyContent>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, j| { select_node(id); push_command!(SHARED_BUFFER, SetJustifyContent, id, j as u32); }); self }
    fn align_items<P: Into<Prop<AlignItems>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, a| { select_node(id); push_command!(SHARED_BUFFER, SetAlignItems, id, a as u32); }); self }
    fn flex_wrap<P: Into<Prop<FlexWrap>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, w| { select_node(id); push_command!(SHARED_BUFFER, SetFlexWrap, id, w as u32); }); self }
    fn align_content<P: Into<Prop<AlignContent>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, ac| { select_node(id); push_command!(SHARED_BUFFER, SetAlignContent, id, ac as u32); }); self }
    fn flex_grow<P: Into<Prop<f32>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, grow| { select_node(id); push_command!(SHARED_BUFFER, SetFlexGrow, id, grow); }); self }
    fn z_index<P: Into<Prop<i32>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, z| { select_node(id); push_command!(SHARED_BUFFER, SetZIndex, id, z); }); self }
    fn padding<P: Into<Prop<(f32,f32,f32,f32)>>>(self, p: P) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, (t, r, b, l)| { 
            select_node(id); 
            push_command!(SHARED_BUFFER, SetPadding, id, t, r, b, l); 
        }); self 
    }

    fn border_radius<P: Into<Prop<f32>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, r| { select_node(id); push_command!(SHARED_BUFFER, SetBorderRadius, id, r); }); self }
    fn on_click<F: FnMut() + 'static>(self, handler: F) -> Self where Self: Sized {
        let id = self.node_id(); select_node(id); push_command!(SHARED_BUFFER, AttachClick, id); 
        CLICK_HANDLERS.with(|handlers| { handlers.borrow_mut().insert(id, Box::new(handler)); }); self
    }
    fn child(self, child_id: u32) -> Self where Self: Sized { select_node(self.node_id()); push_command!(SHARED_BUFFER, AddChild, self.node_id(), child_id); self }
}

pub struct View { pub id: u32 }
impl View {
    pub fn new() -> Self {
        let id = NODE_COUNTER.fetch_add(1, Ordering::SeqCst); track_node(id);
        push_command!(SHARED_BUFFER, CreateNode, id);
        unsafe { LAST_SELECTED_NODE = Some(id); } 
        let v = Self { id }; 
        v.width("auto").height("auto")
         .flex_direction(FlexDirection::Row)
         .justify_content(JustifyContent::FlexStart)
         .align_items(AlignItems::FlexStart)
    }
}
impl BaseView for View { fn node_id(&self) -> u32 { self.id } }

pub struct Text { pub id: u32 }
impl Text {
    pub fn new() -> Self {
        let id = NODE_COUNTER.fetch_add(1, Ordering::SeqCst); track_node(id);
        unsafe {
            push_command!(SHARED_BUFFER, CreateTextNode, id);
            LAST_SELECTED_NODE = Some(id);
        }
        Self { id }
    }
    pub fn value<P: Into<Prop<String>>>(self, p: P) -> Self { apply_prop(self.id, p.into(), |id, s| {
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
    }); self }
    pub fn font_size<P: Into<Prop<f32>>>(self, p: P) -> Self { apply_prop(self.id, p.into(), |id, size| { select_node(id); push_command!(SHARED_BUFFER, SetFontSize, id, size); }); self }
    pub fn font_weight<P: Into<Prop<u16>>>(self, p: P) -> Self { apply_prop(self.id, p.into(), |id, weight| { select_node(id); push_command!(SHARED_BUFFER, SetTextWeight, id, weight); }); self }
    pub fn font_family<P: Into<Prop<String>>>(self, p: P) -> Self { apply_prop(self.id, p.into(), |id, s| {
        select_node(id);
        let len = s.len() as u32;
        unsafe {
            push_command!(SHARED_BUFFER, SetTextFontFamily, id, len);
            let offset = SHARED_BUFFER.command_len as usize;
            if offset + s.len() <= MAX_COMMAND_BYTES {
                SHARED_BUFFER.command_data[offset..offset+s.len()].copy_from_slice(s.as_bytes());
                SHARED_BUFFER.command_len = (offset + s.len()) as u32;
            }
        }
    }); self }
    pub fn text_color<P: Into<Prop<(u8,u8,u8,u8)>>>(self, p: P) -> Self { apply_prop(self.id, p.into(), |id, (r,g,b,a)| { select_node(id); push_command!(SHARED_BUFFER, SetTextColor, id, r, g, b, a); }); self }
}
impl BaseView for Text { fn node_id(&self) -> u32 { self.id } }

pub fn force_layout() { push_command!(SHARED_BUFFER, UpdateLayout); unsafe { ui_force_layout(); } }
