use futures_signals::signal::{Signal, SignalExt};
use std::sync::atomic::{AtomicU32, Ordering};
use std::cell::RefCell;
use std::collections::HashMap;
pub use shared::{FlexDirection, JustifyContent, AlignItems, PositionType, Dimension, Role, ViewType, OpCode, LayoutResult, MAX_COMMAND_BYTES, SharedBuffer};

// --- Command Stream ---
#[no_mangle]
pub static mut SHARED_BUFFER: SharedBuffer = SharedBuffer {
    command_len: 0,
    max_node_id: 0,
    _padding: [0; 2],
    command_data: [0; MAX_COMMAND_BYTES],
    layout_results: [LayoutResult { x: 0.0, y: 0.0, width: 0.0, height: 0.0 }; shared::MAX_NODES],
    dirty_mask: [0; 32],
};

#[no_mangle]
pub extern "C" fn vello_get_shared_buffer_ptr() -> u32 {
    std::ptr::addr_of!(SHARED_BUFFER) as u32
}

static mut LAST_SELECTED_NODE: Option<u32> = None;

fn push_op(op: OpCode, data: &[u8]) {
    unsafe {
        let len = SHARED_BUFFER.command_len as usize;
        if len + 1 + data.len() > MAX_COMMAND_BYTES { return; }
        SHARED_BUFFER.command_data[len] = op as u8;
        if !data.is_empty() { SHARED_BUFFER.command_data[len + 1..len + 1 + data.len()].copy_from_slice(data); }
        SHARED_BUFFER.command_len += 1 + data.len() as u32;
    }
}

fn select_node(id: u32) {
    unsafe {
        if LAST_SELECTED_NODE == Some(id) { return; }
        push_op(OpCode::SelectNode, &id.to_le_bytes());
        LAST_SELECTED_NODE = Some(id);
    }
}

fn track_node(id: u32) { unsafe { if id > SHARED_BUFFER.max_node_id { SHARED_BUFFER.max_node_id = id; } } }
pub fn get_layout(id: u32) -> LayoutResult { unsafe { SHARED_BUFFER.layout_results[id as usize] } }

#[link(wasm_import_module = "env")]
extern "C" { fn ui_force_layout(); }

thread_local! {
    static EXECUTOR: RefCell<Vec<std::pin::Pin<Box<dyn futures_util::future::Future<Output = ()>>>>> = RefCell::new(Vec::new());
    static CLICK_HANDLERS: RefCell<HashMap<u32, Box<dyn FnMut()>>> = RefCell::new(HashMap::new());
}

#[no_mangle]
pub extern "C" fn vello_view_tick() {
    unsafe { 
        LAST_SELECTED_NODE = None; 
        // 重置脏位图
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
    vello_view_tick();
}

pub enum Prop<T> { Static(T), Dynamic(Box<dyn Signal<Item = T> + Unpin + 'static>) }
impl<T: 'static + Copy> From<T> for Prop<T> { fn from(v: T) -> Self { Prop::Static(v) } }
impl From<&str> for Prop<Dimension> { fn from(v: &str) -> Self { Prop::Static(Dimension::from(v)) } }
impl From<f32> for Prop<Dimension> { fn from(v: f32) -> Self { Prop::Static(Dimension::from(v)) } }
impl From<i32> for Prop<Dimension> { fn from(v: i32) -> Self { Prop::Static(Dimension::from(v)) } }
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
            push_op(OpCode::SetColorCompact, &[r as u8, g as u8, b as u8, 255]);
        }); self 
    }
    fn width<P: Into<Prop<Dimension>>>(self, p: P) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, dim| {
            select_node(id);
            let (t, v) = match dim { Dimension::Auto => (0u8, 0.0f32), Dimension::Pixels(x) => (1, x), Dimension::Percent(x) => (2, x) };
            let mut data = [0u8; 5]; data[0] = t; data[1..5].copy_from_slice(&v.to_le_bytes());
            push_op(OpCode::SetWidthCompact, &data);
        }); self 
    }
    fn height<P: Into<Prop<Dimension>>>(self, p: P) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, dim| {
            select_node(id);
            let (t, v) = match dim { Dimension::Auto => (0u8, 0.0f32), Dimension::Pixels(x) => (1, x), Dimension::Percent(x) => (2, x) };
            let mut data = [0u8; 5]; data[0] = t; data[1..5].copy_from_slice(&v.to_le_bytes());
            push_op(OpCode::SetHeightCompact, &data);
        }); self 
    }
    fn flex_direction<P: Into<Prop<FlexDirection>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, dir| { select_node(id); push_op(OpCode::SetFlexDirection, &(dir as u32).to_le_bytes()); }); self }
    fn justify_content<P: Into<Prop<JustifyContent>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, j| { select_node(id); push_op(OpCode::SetJustifyContent, &(j as u32).to_le_bytes()); }); self }
    fn align_items<P: Into<Prop<AlignItems>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, a| { select_node(id); push_op(OpCode::SetAlignItems, &(a as u32).to_le_bytes()); }); self }
    fn position<P: Into<Prop<PositionType>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, pos| { select_node(id); push_op(OpCode::SetPosition, &(pos as u32).to_le_bytes()); }); self }
    fn flex_grow<P: Into<Prop<f32>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, grow| { select_node(id); push_op(OpCode::SetFlexGrow, &grow.to_le_bytes()); }); self }
    fn z_index<P: Into<Prop<i32>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, z| { select_node(id); push_op(OpCode::SetZIndex, &z.to_le_bytes()); }); self }
    fn inset<P: Into<Prop<(f32,f32,f32,f32)>>>(self, p: P) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, (t, r, b, l)| { 
            select_node(id);
            let mut data = [0u8; 16]; 
            data[0..4].copy_from_slice(&t.to_le_bytes()); data[4..8].copy_from_slice(&r.to_le_bytes()); 
            data[8..12].copy_from_slice(&b.to_le_bytes()); data[12..16].copy_from_slice(&l.to_le_bytes());
            let mut full_data = [0u8; 20];
            full_data[0..4].copy_from_slice(&id.to_le_bytes());
            full_data[4..20].copy_from_slice(&data);
            push_op(OpCode::SetInset, &full_data); 
        }); self 
    }
    fn padding<P: Into<Prop<(f32,f32,f32,f32)>>>(self, p: P) -> Self where Self: Sized { 
        apply_prop(self.node_id(), p.into(), |id, (t, r, b, l)| { 
            select_node(id); 
            let mut data = [0u8; 20]; 
            data[0..4].copy_from_slice(&id.to_le_bytes()); 
            data[4..8].copy_from_slice(&t.to_le_bytes()); 
            data[8..12].copy_from_slice(&r.to_le_bytes()); 
            data[12..16].copy_from_slice(&b.to_le_bytes()); 
            data[16..20].copy_from_slice(&l.to_le_bytes()); 
            push_op(OpCode::SetPadding, &data); 
        }); self 
    }

    fn border_radius<P: Into<Prop<f32>>>(self, p: P) -> Self where Self: Sized { apply_prop(self.node_id(), p.into(), |id, r| { select_node(id); push_op(OpCode::SetBorderRadius, &r.to_le_bytes()); }); self }
    fn on_click<F: FnMut() + 'static>(self, handler: F) -> Self where Self: Sized {
        let id = self.node_id(); select_node(id); push_op(OpCode::AttachClick, &[]); 
        CLICK_HANDLERS.with(|handlers| { handlers.borrow_mut().insert(id, Box::new(handler)); }); self
    }
    fn child(self, child_id: u32) -> Self where Self: Sized { select_node(self.node_id()); push_op(OpCode::AddChild, &child_id.to_le_bytes()); self }
}

pub struct View { pub id: u32 }
impl View {
    pub fn new() -> Self {
        let id = NODE_COUNTER.fetch_add(1, Ordering::SeqCst); track_node(id);
        push_op(OpCode::CreateNode, &id.to_le_bytes());
        unsafe { LAST_SELECTED_NODE = Some(id); } 
        let v = Self { id }; v.width("auto").height("auto") 
    }
}
impl BaseView for View { fn node_id(&self) -> u32 { self.id } }

pub struct Text { pub id: u32 }
impl Text {
    pub fn new() -> Self {
        let id = NODE_COUNTER.fetch_add(1, Ordering::SeqCst); track_node(id);
        push_op(OpCode::CreateNode, &id.to_le_bytes());
        unsafe { LAST_SELECTED_NODE = Some(id); }
        push_op(OpCode::SetViewType, &(ViewType::Text as u32).to_le_bytes());
        Self { id }
    }
    pub fn value<P: Into<Prop<String>>>(self, p: P) -> Self { apply_prop(self.id, p.into(), |id, s| { select_node(id); let mut data = [0u8; 4]; data.copy_from_slice(&(s.len() as u32).to_le_bytes()); let mut full = data.to_vec(); full.extend_from_slice(s.as_bytes()); push_op(OpCode::SetText, &full); }); self }
}
impl BaseView for Text { fn node_id(&self) -> u32 { self.id } }

pub fn force_layout() { push_op(OpCode::UpdateLayout, &[]); unsafe { ui_force_layout(); } }

#[macro_export]
macro_rules! rsx {
    ($tag:ident { $($inner:tt)* }) => {{ use $crate::BaseView; let mut n = $crate::$tag::new(); $crate::rsx!(@internal n { $($inner)* }); n.node_id() }};
    (@internal $n:ident { $prop:ident : ~ $val:expr ; $($rest:tt)* }) => { let $n = $n.$prop($val.signal().sig()); $crate::rsx!(@internal $n { $($rest)* }); };
    (@internal $n:ident { $prop:ident : $val:expr ; $($rest:tt)* }) => { let $n = $n.$prop($val); $crate::rsx!(@internal $n { $($rest)* }); };
    (@internal $n:ident { on_click : $val:expr ; $($rest:tt)* }) => { let $n = $n.on_click($val); $crate::rsx!(@internal $n { $($rest)* }); };
    (@internal $n:ident { }) => {};
}
