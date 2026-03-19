use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use taffy::prelude::*;
use vello::peniko::{Color, Fill};
use vello::{Renderer, RendererOptions, Scene, util::RenderContext};
use kurbo::{Affine, Rect as KRect, RoundedRect, Vec2, Point};
pub use shared::{FlexDirection, JustifyContent, AlignItems, PositionType, Role, ViewType, OpCode, LayoutResult, MAX_COMMAND_BYTES, MAX_NODES, SharedBuffer};

#[cfg(not(target_arch = "wasm32"))]
uniffi::setup_scaffolding!();

use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, WindowHandle};
#[cfg(target_os = "android")] use raw_window_handle::AndroidNdkWindowHandle;
#[cfg(target_os = "ios")] use raw_window_handle::{UiKitDisplayHandle, UiKitWindowHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub struct SurfaceId(pub u64);

pub struct SafeWindowHandle {
    #[cfg(target_os = "android")] android_window: Option<std::ptr::NonNull<ndk_sys::ANativeWindow>>,
    #[allow(dead_code)] raw_window_handle: RawWindowHandle,
    #[allow(dead_code)] raw_display_handle: RawDisplayHandle,
}

impl SafeWindowHandle {
    #[cfg(target_os = "android")]
    pub fn new_android(surface_ptr: u64) -> Self {
        let ptr = std::ptr::NonNull::new(surface_ptr as *mut ndk_sys::ANativeWindow).expect("Null");
        Self { android_window: Some(ptr), raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(ptr.cast())), raw_display_handle: RawDisplayHandle::Android(raw_window_handle::AndroidDisplayHandle::new()) }
    }
    #[cfg(target_os = "ios")]
    pub fn new_ios(surface_ptr: u64) -> Self {
        Self { #[cfg(target_os = "android")] android_window: None, raw_window_handle: RawWindowHandle::UiKit(raw_window_handle::UiKitWindowHandle::new(std::ptr::NonNull::new(surface_ptr as *mut _).unwrap())), raw_display_handle: RawDisplayHandle::UiKit(raw_window_handle::UiKitDisplayHandle::new()) }
    }
}

#[cfg(target_os = "android")]
impl Drop for SafeWindowHandle { fn drop(&mut self) { if let Some(ptr) = self.android_window { unsafe { ndk_sys::ANativeWindow_release(ptr.as_ptr()); } } } }
impl HasWindowHandle for SafeWindowHandle { fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> { unsafe { Ok(WindowHandle::borrow_raw(self.raw_window_handle)) } } }
impl HasDisplayHandle for SafeWindowHandle { fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> { unsafe { Ok(DisplayHandle::borrow_raw(self.raw_display_handle)) } } }
unsafe impl Send for SafeWindowHandle {}
unsafe impl Sync for SafeWindowHandle {}

pub struct EngineState {
    pub context: RenderContext, pub renderer: Renderer, pub shared_state: Arc<Mutex<SharedState>>,
    #[cfg(feature = "wasm3-support")] pub tick_fn: wasm3::Function<'static, (), ()>,
    #[cfg(feature = "wasm3-support")] pub on_click_fn: wasm3::Function<'static, (u32,), ()>,
    #[cfg(feature = "wasm3-support")] pub _rt: wasm3::Runtime,
    #[cfg(feature = "wasm3-support")] pub shared_buffer_ptr: u32,
    pub blit_bind_group_layout: vello::wgpu::BindGroupLayout, pub sampler: vello::wgpu::Sampler, pub blit_shader: vello::wgpu::ShaderModule,
}
unsafe impl Send for EngineState {}
unsafe impl Sync for EngineState {}

pub struct SurfaceState { pub surface: vello::util::RenderSurface<'static>, pub blit_pipeline: vello::wgpu::RenderPipeline, pub offscreen_texture: Option<(vello::wgpu::Texture, vello::wgpu::BindGroup)>, #[allow(dead_code)] pub window_handle: Option<Arc<SafeWindowHandle>> }
unsafe impl Send for SurfaceState {}
unsafe impl Sync for SurfaceState {}

pub struct ViewNode { pub taffy_node: NodeId, pub color: Color, pub children: Vec<u32>, pub z_index: i32, pub label: String, pub text: String, pub font_size: f32, pub border_radius: f32, pub role: Role, pub view_type: ViewType, pub has_click: bool, pub padding: (f32, f32, f32, f32) }
pub struct SharedState { pub taffy: TaffyTree<()>, pub nodes: HashMap<u32, ViewNode>, pub root_id: Option<u32>, pub click_listeners: Vec<u32>, pub font_data: Option<Vec<u8>> }
unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

impl SharedState {
    pub fn new() -> Self { Self { taffy: TaffyTree::new(), nodes: HashMap::new(), root_id: None, click_listeners: vec![], font_data: None } }
    pub fn create_node(&mut self, id: u32) {
        let taffy_node = self.taffy.new_leaf(Style::default()).unwrap();
        self.nodes.insert(id, ViewNode { taffy_node, color: Color::WHITE, children: vec![], z_index: 0, label: String::new(), text: String::new(), font_size: 16.0, border_radius: 0.0, role: Role::None, view_type: ViewType::Container, has_click: false, padding: (0.0, 0.0, 0.0, 0.0) });
        if self.root_id.is_none() { self.root_id = Some(id); }
    }
    pub fn set_view_type(&mut self, id: u32, vt: u32) { if let Some(node) = self.nodes.get_mut(&id) { node.view_type = match vt { 1 => ViewType::Text, 2 => ViewType::Button, _ => ViewType::Container }; } }
    pub fn set_text(&mut self, id: u32, text: String) { if let Some(node) = self.nodes.get_mut(&id) { node.text = text; } }
    pub fn set_font_size(&mut self, id: u32, size: f32) { if let Some(node) = self.nodes.get_mut(&id) { node.font_size = size; } }
    pub fn set_color(&mut self, id: u32, r: u8, g: u8, b: u8) { if let Some(node) = self.nodes.get_mut(&id) { node.color = Color::from_rgb8(r, g, b); } }
    pub fn set_width(&mut self, id: u32, dt: u32, v: f32) { if let Some(node) = self.nodes.get(&id) { let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); s.size.width = match dt { 1 => taffy::style::Dimension::length(v), 2 => taffy::style::Dimension::percent(v / 100.0), _ => taffy::style::Dimension::auto() }; self.taffy.set_style(node.taffy_node, s).unwrap(); } }
    pub fn set_height(&mut self, id: u32, dt: u32, v: f32) { if let Some(node) = self.nodes.get(&id) { let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); s.size.height = match dt { 1 => taffy::style::Dimension::length(v), 2 => taffy::style::Dimension::percent(v / 100.0), _ => taffy::style::Dimension::auto() }; self.taffy.set_style(node.taffy_node, s).unwrap(); } }
    pub fn set_inset(&mut self, id: u32, t: f32, r: f32, b: f32, l: f32) { if let Some(node) = self.nodes.get(&id) { let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); s.inset.top = LengthPercentage::percent(t / 100.0).into(); s.inset.right = LengthPercentage::percent(r / 100.0).into(); s.inset.bottom = LengthPercentage::percent(b / 100.0).into(); s.inset.left = LengthPercentage::percent(l / 100.0).into(); self.taffy.set_style(node.taffy_node, s).unwrap(); } }
    pub fn set_position(&mut self, id: u32, p: u32) { if let Some(node) = self.nodes.get(&id) { let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); s.position = if p == 1 { Position::Absolute } else { Position::Relative }; self.taffy.set_style(node.taffy_node, s).unwrap(); } }
    pub fn set_flex_direction(&mut self, id: u32, dir: u32) { if let Some(node) = self.nodes.get(&id) { let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); s.flex_direction = match dir { 1 => taffy::prelude::FlexDirection::Column, 2 => taffy::prelude::FlexDirection::RowReverse, 3 => taffy::prelude::FlexDirection::ColumnReverse, _ => taffy::prelude::FlexDirection::Row }; self.taffy.set_style(node.taffy_node, s).unwrap(); } }
    pub fn set_justify_content(&mut self, id: u32, j: u32) { if let Some(node) = self.nodes.get(&id) { let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); s.justify_content = Some(match j { 1 => taffy::prelude::JustifyContent::Center, 2 => taffy::prelude::JustifyContent::FlexEnd, 3 => taffy::prelude::JustifyContent::SpaceBetween, 4 => taffy::prelude::JustifyContent::SpaceAround, 5 => taffy::prelude::JustifyContent::SpaceEvenly, _ => taffy::prelude::JustifyContent::FlexStart }); self.taffy.set_style(node.taffy_node, s).unwrap(); } }
    pub fn set_align_items(&mut self, id: u32, a: u32) { if let Some(node) = self.nodes.get(&id) { let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); s.align_items = Some(match a { 1 => taffy::prelude::AlignItems::Center, 2 => taffy::prelude::AlignItems::FlexEnd, 3 => taffy::prelude::AlignItems::Stretch, _ => taffy::prelude::AlignItems::FlexStart }); self.taffy.set_style(node.taffy_node, s).unwrap(); } }
    pub fn set_flex_grow(&mut self, id: u32, grow: f32) { if let Some(node) = self.nodes.get(&id) { let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); s.flex_grow = grow; self.taffy.set_style(node.taffy_node, s).unwrap(); } }
    pub fn set_z_index(&mut self, id: u32, z: i32) { if let Some(node) = self.nodes.get_mut(&id) { node.z_index = z; } }
    pub fn set_border_radius(&mut self, id: u32, r: f32) { if let Some(node) = self.nodes.get_mut(&id) { node.border_radius = r; } }
    pub fn set_padding(&mut self, id: u32, t: f32, r: f32, b: f32, l: f32) { if let Some(node) = self.nodes.get(&id) { let mut s = self.taffy.style(node.taffy_node).unwrap().clone(); s.padding.top = LengthPercentage::length(t).into(); s.padding.right = LengthPercentage::length(r).into(); s.padding.bottom = LengthPercentage::length(b).into(); s.padding.left = LengthPercentage::length(l).into(); self.taffy.set_style(node.taffy_node, s).unwrap(); } }
    pub fn attach_click(&mut self, id: u32) { self.click_listeners.push(id); if let Some(node) = self.nodes.get_mut(&id) { node.has_click = true; } }
    pub fn add_child(&mut self, pid: u32, cid: u32) { let c_tn = self.nodes.get(&cid).map(|n| n.taffy_node); let p_tn = self.nodes.get(&pid).map(|n| n.taffy_node); if let (Some(ptn), Some(ctn)) = (p_tn, c_tn) { if let Some(parent) = self.nodes.get_mut(&pid) { parent.children.push(cid); } self.taffy.add_child(ptn, ctn).unwrap(); } }
    pub fn set_font_data(&mut self, data: Vec<u8>) { self.font_data = Some(data); }
}

pub fn render_node_recursive(id: u32, state: &SharedState, scene: &mut Scene, parent_pos: Vec2) {
    if let Some(node) = state.nodes.get(&id) {
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let global_pos = parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);
        let rect = KRect::from_origin_size((global_pos.x, global_pos.y), (layout.size.width as f64, layout.size.height as f64));
        if node.border_radius > 0.0 {
            let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
            scene.fill(Fill::NonZero, Affine::IDENTITY, node.color, None, &rounded);
        } else {
            scene.fill(Fill::NonZero, Affine::IDENTITY, node.color, None, &rect);
        }
        for &child_id in &node.children { render_node_recursive(child_id, state, scene, global_pos); }
    }
}

pub fn hit_test_recursive(id: u32, point: Vec2, nodes: &HashMap<u32, ViewNode>, taffy: &TaffyTree<()>, parent_pos: Vec2, listeners: &[u32]) -> Option<u32> {
    if let Some(node) = nodes.get(&id) {
        let layout = taffy.layout(node.taffy_node).unwrap();
        let global_pos = parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);
        let rect = KRect::from_origin_size((global_pos.x, global_pos.y), (layout.size.width as f64, layout.size.height as f64));
        for &child_id in node.children.iter().rev() { if let Some(hit) = hit_test_recursive(child_id, point, nodes, taffy, global_pos, listeners) { return Some(hit); } }
        if rect.contains(Point::new(point.x, point.y)) && listeners.contains(&id) { return Some(id); }
    }
    None
}

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

pub fn process_command_stream(state: &Arc<Mutex<SharedState>>, command_data: &[u8]) -> anyhow::Result<()> {
    let mut offset = 0; let mut s = state.lock().unwrap(); let mut cur_id: Option<u32> = None;
    while offset < command_data.len() {
        let op_byte = command_data[offset]; offset += 1;
        let op = match OpCode::from_u8(op_byte) {
            Some(o) => o,
            None => { log::warn!("Unknown opcode: {}", op_byte); continue; }
        };
        shared::dispatch_op!(op, command_data, offset, handle_op, s, cur_id, offset, command_data);
    }
    Ok(())
}

pub fn process_commands(memory: &mut [u8], buffer_ptr: u32, state: &Arc<Mutex<SharedState>>) -> anyhow::Result<()> {
    let bs = buffer_ptr as usize; let clen = u32::from_le_bytes(memory[bs..bs+4].try_into()?);
    if clen == 0 { return Ok(()); }
    let _ = process_command_stream(state, &memory[bs+16 .. bs+16+clen as usize]);
    memory[bs..bs + 4].copy_from_slice(&0u32.to_le_bytes()); Ok(())
}

pub fn sync_layout_to_wasm(memory: &mut [u8], buffer_ptr: u32, state: &SharedState) -> anyhow::Result<()> {
    let bs = buffer_ptr as usize;
    let ls = bs + 16 + MAX_COMMAND_BYTES;
    let ms = ls + (MAX_NODES * 16); 
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

#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Object))]
pub struct VelloHost { engine: Arc<Mutex<Option<EngineState>>>, active_surface_id: Mutex<Option<SurfaceId>>, surfaces: Mutex<HashMap<u64, SurfaceState>>, next_surface_id: AtomicU64 }

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
impl VelloHost {
    #[cfg_attr(not(target_arch = "wasm32"), uniffi::constructor)] pub fn new() -> Arc<Self> { Arc::new(Self { engine: Arc::new(Mutex::new(None)), active_surface_id: Mutex::new(None), surfaces: Mutex::new(HashMap::new()), next_surface_id: AtomicU64::new(1) }) }
    pub fn resize_native(&self, width: u32, height: u32) { if let Some(id) = *self.active_surface_id.lock().unwrap() { let mut surfs = self.surfaces.lock().unwrap(); if let Some(s) = surfs.get_mut(&id.0) { if let Some(e) = &mut *self.engine.lock().unwrap() { e.context.resize_surface(&mut s.surface, width, height); render_frame(e, s); } } } }
    pub fn stop_native(&self) { if let Some(id) = self.active_surface_id.lock().unwrap().take() { self.surfaces.lock().unwrap().remove(&id.0); } }
    pub fn tick(&self) {
        let active_id = { *self.active_surface_id.lock().unwrap() };
        if let Some(id) = active_id {
            let mut eg = self.engine.lock().unwrap();
            let mut surfs = self.surfaces.lock().unwrap();
            if let (Some(e), Some(s)) = (&mut *eg, surfs.get_mut(&id.0)) {
                #[cfg(feature = "wasm3-support")] {
                    let mem = unsafe { &mut *e._rt.memory_mut() };
                    let _ = process_commands(mem, e.shared_buffer_ptr, &e.shared_state);
                    if let Err(err) = e.tick_fn.call() { log::error!("Wasm tick failed: {}", err); }
                    let _ = sync_layout_to_wasm(mem, e.shared_buffer_ptr, &e.shared_state.lock().unwrap());
                }
                render_frame(e, s); 
            }
        }
    }
    pub fn on_touch(&self, x: f32, y: f32) {
        if let Some(e) = &*self.engine.lock().unwrap() {
            let mp = Vec2::new(x as f64, y as f64);
            let hit = { let sg = e.shared_state.lock().unwrap(); sg.root_id.and_then(|rid| hit_test_recursive(rid, mp, &sg.nodes, &sg.taffy, Vec2::ZERO, &sg.click_listeners)) };
            if let Some(_target_id) = hit { 
                #[cfg(feature = "wasm3-support")] { 
                    if let Err(err) = e.on_click_fn.call(_target_id) { log::error!("Wasm click failed: {}", err); }
                } 
            }
        }
    }
    pub fn is_initialized(&self) -> bool { self.active_surface_id.lock().unwrap().is_some() }
    pub async fn prepare_engine(&self, ddir: String) {
        if self.engine.lock().unwrap().is_some() { return; }
        if let Ok(e) = setup_engine(ddir, self.engine.clone()).await {
            let mut guard = self.engine.lock().unwrap();
            if guard.is_none() { *guard = Some(e); }
        }
    }
    pub async fn init_native(&self, surface_ptr: u64, ddir: String, w: u32, h: u32) {
        #[cfg(target_os = "android")] let sh = Arc::new(SafeWindowHandle::new_android(surface_ptr));
        #[cfg(target_os = "ios")] let sh = Arc::new(SafeWindowHandle::new_ios(surface_ptr));
        #[cfg(any(target_os = "android", target_os = "ios"))] self.setup(vello::wgpu::SurfaceTarget::from(sh.clone()), ddir, w, h, Some(sh)).await;
        #[cfg(not(any(target_os = "android", target_os = "ios")))] { let _ = (surface_ptr, ddir, w, h); }
    }
}

impl VelloHost {
    pub async fn setup(&self, target: vello::wgpu::SurfaceTarget<'static>, ddir: String, width: u32, height: u32, handle: Option<Arc<SafeWindowHandle>>) {
        let mut engine_ready = self.engine.lock().unwrap().is_some();
        if !engine_ready {
            if let Ok(e) = setup_engine(ddir, self.engine.clone()).await {
                let mut guard = self.engine.lock().unwrap();
                if guard.is_none() { *guard = Some(e); }
                engine_ready = true;
            }
        }
        if !engine_ready { return; }
        let engine_opt = { self.engine.lock().unwrap().take() };
        if let Some(mut e) = engine_opt {
            if let Ok(surface) = e.context.create_surface(target, width, height, vello::wgpu::PresentMode::AutoVsync).await {
                let dev = &e.context.devices[surface.dev_id].device;
                let bl = dev.create_pipeline_layout(&vello::wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&e.blit_bind_group_layout], push_constant_ranges: &[] });
                let blit_p = dev.create_render_pipeline(&vello::wgpu::RenderPipelineDescriptor { label: None, layout: Some(&bl), vertex: vello::wgpu::VertexState { module: &e.blit_shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() }, fragment: Some(vello::wgpu::FragmentState { module: &e.blit_shader, entry_point: Some("fs_main"), targets: &[Some(vello::wgpu::ColorTargetState { format: surface.config.format, blend: Some(vello::wgpu::BlendState::REPLACE), write_mask: vello::wgpu::ColorWrites::ALL })], compilation_options: Default::default() }), primitive: vello::wgpu::PrimitiveState::default(), depth_stencil: None, multisample: vello::wgpu::MultisampleState::default(), multiview: None, cache: None });
                let nid = self.next_surface_id.fetch_add(1, Ordering::SeqCst);
                let mut ss = SurfaceState { surface, blit_pipeline: blit_p, offscreen_texture: None, window_handle: handle };
                render_frame(&mut e, &mut ss);
                self.surfaces.lock().unwrap().insert(nid, ss);
                *self.active_surface_id.lock().unwrap() = Some(SurfaceId(nid));
            }
            let mut guard = self.engine.lock().unwrap();
            *guard = Some(e);
        }
    }
    pub fn get_state(&self) -> std::sync::MutexGuard<'_, Option<EngineState>> { self.engine.lock().unwrap() }
    pub fn get_state_mut(&self) -> std::sync::MutexGuard<'_, Option<EngineState>> { self.engine.lock().unwrap() }
    pub fn apply_commands(&self, command_data: &[u8]) { if let Some(s) = &*self.engine.lock().unwrap() { let _ = process_command_stream(&s.shared_state, command_data); } }
}

async fn setup_engine(ddir: String, _es: Arc<Mutex<Option<EngineState>>>) -> anyhow::Result<EngineState> {
    let mut context = RenderContext::new(); 
    let dev_id = context.device(None).await.ok_or_else(|| anyhow::anyhow!("No device found"))?;
    let dev = &context.devices[dev_id].device;
    let blit_shader = dev.create_shader_module(vello::wgpu::ShaderModuleDescriptor { label: Some("Blit Shader"), source: vello::wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()) });
    let blit_bl = dev.create_bind_group_layout(&vello::wgpu::BindGroupLayoutDescriptor { label: None, entries: &[vello::wgpu::BindGroupLayoutEntry { binding: 0, visibility: vello::wgpu::ShaderStages::FRAGMENT, ty: vello::wgpu::BindingType::Texture { sample_type: vello::wgpu::TextureSampleType::Float { filterable: true }, view_dimension: vello::wgpu::TextureViewDimension::D2, multisampled: false }, count: None }, vello::wgpu::BindGroupLayoutEntry { binding: 1, visibility: vello::wgpu::ShaderStages::FRAGMENT, ty: vello::wgpu::BindingType::Sampler(vello::wgpu::SamplerBindingType::Filtering), count: None }] });
    let sampler = dev.create_sampler(&vello::wgpu::SamplerDescriptor { mag_filter: vello::wgpu::FilterMode::Linear, min_filter: vello::wgpu::FilterMode::Linear, ..Default::default() });
    let renderer = Renderer::new(dev, RendererOptions { antialiasing_support: vello::AaSupport::all(), pipeline_cache: None, num_init_threads: None, use_cpu: false }).map_err(|e| anyhow::anyhow!("Failed to create renderer: {}", e))?;
    let state = Arc::new(Mutex::new(SharedState::new()));
    #[cfg(feature = "wasm3-support")] {
        let wasm_path = format!("{}/guest.wasm", ddir); 
        let wasm = std::fs::read(&wasm_path).or_else(|_| std::fs::read("guest.wasm")).map_err(|e| anyhow::anyhow!("Failed to read WASM: {}", e))?;
        let env = wasm3::Environment::new().map_err(|e| anyhow::anyhow!("Environment failed: {}", e))?; 
        let rt = env.create_runtime(1024 * 2048).map_err(|e| anyhow::anyhow!("Runtime failed: {}", e))?;
        let mut module = rt.load_module(env.parse_module(wasm).map_err(|e| anyhow::anyhow!("Parse failed: {}", e))?).map_err(|e| anyhow::anyhow!("Load failed: {}", e))?;
        let bptr = module.find_function::<(), u32>("vello_get_shared_buffer_ptr").map_err(|e| anyhow::anyhow!("Func not found: {}", e))?.call().map_err(|e| anyhow::anyhow!("Call failed: {}", e))?;
        let s_inner = state.clone();
        let _ = module.link_closure("env", "ui_force_layout", move |ctx, ()| { let mem = unsafe { &mut *ctx.memory_mut() }; let _ = process_commands(mem, bptr, &s_inner); Ok(()) });
        let main_fn = module.find_function::<(), ()>("main").or_else(|_| module.find_function::<(), ()>("_main")).map_err(|e| anyhow::anyhow!("Main not found"))?;
        let get_hash_fn = module.find_function::<(), u64>("vello_get_protocol_hash").map_err(|e| anyhow::anyhow!("vello_get_protocol_hash not found"))?;
        let guest_hash = get_hash_fn.call().map_err(|e| anyhow::anyhow!("Failed to call get_hash"))?;
        if guest_hash != shared::PROTOCOL_HASH { return Err(anyhow::anyhow!("Protocol mismatch! Host: {}, Guest: {}", shared::PROTOCOL_HASH, guest_hash)); }
        let tick_fn = module.find_function::<(), ()>("guest_tick").or_else(|_| module.find_function::<(), ()>("vello_tick")).or_else(|_| module.find_function::<(), ()>("_guest_tick")).map_err(|e| anyhow::anyhow!("Tick func not found"))?;
        let on_click_fn = module.find_function::<(u32,), ()>("on_node_click").or_else(|_| module.find_function::<(u32,), ()>("_on_node_click")).map_err(|e| anyhow::anyhow!("OnClick func not found"))?;
        let _ = main_fn.call(); let memory = unsafe { &mut *rt.memory_mut() }; let _ = process_commands(memory, bptr, &state);
        Ok(EngineState { context, renderer, shared_state: state, tick_fn: unsafe { std::mem::transmute(tick_fn) }, on_click_fn: unsafe { std::mem::transmute(on_click_fn) }, _rt: rt, shared_buffer_ptr: bptr, blit_bind_group_layout: blit_bl, sampler, blit_shader })
    }
    #[cfg(not(feature = "wasm3-support"))] { Ok(EngineState { context, renderer, shared_state: state, blit_bind_group_layout: blit_bl, sampler, blit_shader }) }
}

fn render_frame(e: &mut EngineState, s: &mut SurfaceState) {
    let w = s.surface.config.width; let h = s.surface.config.height; if w == 0 || h == 0 { return; }
    let rid = { let mut g = e.shared_state.lock().unwrap(); g.root_id.map(|id| { if let Some(rn) = g.nodes.get(&id).map(|n| n.taffy_node) { let _ = g.taffy.compute_layout(rn, taffy::prelude::Size { width: AvailableSpace::Definite(w as f32), height: AvailableSpace::Definite(h as f32) }); } id }) };
    let mut scene = Scene::new(); if let Some(id) = rid { let g = e.shared_state.lock().unwrap(); render_node_recursive(id, &g, &mut scene, Vec2::ZERO); }
    if s.offscreen_texture.as_ref().map_or(true, |(t, _)| t.width() != w || t.height() != h) {
        let texture = e.context.devices[s.surface.dev_id].device.create_texture(&vello::wgpu::TextureDescriptor { label: None, size: vello::wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: vello::wgpu::TextureDimension::D2, format: vello::wgpu::TextureFormat::Rgba8Unorm, usage: vello::wgpu::TextureUsages::STORAGE_BINDING | vello::wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let bg = e.context.devices[s.surface.dev_id].device.create_bind_group(&vello::wgpu::BindGroupDescriptor { label: None, layout: &e.blit_bind_group_layout, entries: &[vello::wgpu::BindGroupEntry { binding: 0, resource: vello::wgpu::BindingResource::TextureView(&texture.create_view(&Default::default())) }, vello::wgpu::BindGroupEntry { binding: 1, resource: vello::wgpu::BindingResource::Sampler(&e.sampler) }] });
        s.offscreen_texture = Some((texture, bg));
    }
    let (off_t, blit_bg) = s.offscreen_texture.as_ref().unwrap();
    e.renderer.render_to_texture(&e.context.devices[s.surface.dev_id].device, &e.context.devices[s.surface.dev_id].queue, &scene, &off_t.create_view(&Default::default()), &vello::RenderParams { base_color: Color::BLACK, width: w, height: h, antialiasing_method: vello::AaConfig::Area }).unwrap();
    if let Ok(st) = s.surface.surface.get_current_texture() {
        let mut enc = e.context.devices[s.surface.dev_id].device.create_command_encoder(&Default::default());
        { 
            let mut rp = enc.begin_render_pass(&vello::wgpu::RenderPassDescriptor { label: None, color_attachments: &[Some(vello::wgpu::RenderPassColorAttachment { view: &st.texture.create_view(&Default::default()), resolve_target: None, ops: vello::wgpu::Operations { load: vello::wgpu::LoadOp::Clear(vello::wgpu::Color::TRANSPARENT), store: vello::wgpu::StoreOp::Store }, depth_slice: None })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None }); rp.set_pipeline(&s.blit_pipeline); rp.set_bind_group(0, blit_bg, &[]); rp.draw(0..3, 0..1); 
        }
        e.context.devices[s.surface.dev_id].queue.submit(Some(enc.finish())); st.present();
    }
}
