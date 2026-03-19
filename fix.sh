cat <<'EOF' > crates/host-core/src/lib.rs
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use taffy::prelude::*;
use vello::peniko::{Color, Brush, Blob, Fill, FontData};
use vello::{Renderer, RendererOptions, Scene, util::RenderContext};
use vello::Glyph;
use kurbo::{Affine, Rect as KRect, RoundedRect, Vec2, Point};
use skrifa::{MetadataProvider, FontRef, instance::Size, instance::LocationRef};
pub use shared::{FlexDirection, JustifyContent, AlignItems, PositionType, Role, ViewType, OpCode, LayoutResult, MAX_COMMAND_BYTES, MAX_NODES, SharedBuffer};

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
    pub fn new_android(ptr: u64) -> Self {
        let ptr = std::ptr::NonNull::new(ptr as *mut ndk_sys::ANativeWindow).expect("Null");
        Self { android_window: Some(ptr), raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(ptr.cast())), raw_display_handle: RawDisplayHandle::Android(raw_window_handle::AndroidDisplayHandle::new()) }
    }
    #[cfg(target_os = "ios")]
    pub fn new_ios(ptr: u64) -> Self {
        Self { #[cfg(target_os = "android")] android_window: None, raw_window_handle: RawWindowHandle::UiKit(raw_window_handle::UiKitWindowHandle::new(std::ptr::NonNull::new(ptr as *mut _).unwrap())), raw_display_handle: RawDisplayHandle::UiKit(raw_window_handle::UiKitDisplayHandle::new()) }
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
    pub fn attach_click(&mut self, id: u32) { self.click_listeners.push(id); }
    pub fn add_child(&mut self, pid: u32, cid: u32) { let c_tn = self.nodes.get(&cid).map(|n| n.taffy_node); let p_tn = self.nodes.get(&pid).map(|n| n.taffy_node); if let (Some(ptn), Some(ctn)) = (p_tn, c_tn) { self.nodes.get_mut(&pid).unwrap().children.push(cid); self.taffy.add_child(ptn, ctn).unwrap(); } }
}

pub fn render_node_recursive(id: u32, state: &SharedState, scene: &mut Scene, parent_pos: Vec2) {
    if let Some(node) = state.nodes.get(&id) {
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let global_pos = parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);
        let rect = KRect::from_origin_size((global_pos.x, global_pos.y), (layout.size.width as f64, layout.size.height as f64));
        scene.fill(Fill::NonZero, Affine::IDENTITY, node.color, None, &rect);
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

pub fn process_command_stream(state: &Arc<Mutex<SharedState>>, command_data: &[u8]) -> anyhow::Result<()> {
    let mut offset = 0; let mut s = state.lock().unwrap(); let mut cur_id: Option<u32> = None;
    while offset < command_data.len() {
        let op = command_data[offset]; offset += 1;
        match op {
            1 => { let id = u32::from_le_bytes(command_data[offset..offset+4].try_into()?); s.create_node(id); cur_id = Some(id); offset += 4; },
            22 => { cur_id = Some(u32::from_le_bytes(command_data[offset..offset+4].try_into()?)); offset += 4; },
            23 => { if let Some(id) = cur_id { s.set_color(id, command_data[offset], command_data[offset+1], command_data[offset+2]); } offset += 4; },
            24 | 25 => { if let Some(id) = cur_id { let t = command_data[offset] as u32; let v = f32::from_le_bytes(command_data[offset+1..offset+5].try_into()?); if op == 24 { s.set_width(id, t, v); } else { s.set_height(id, t, v); } } offset += 5; },
            10 => { let id = u32::from_le_bytes(command_data[offset..offset+4].try_into()?); let t = f32::from_le_bytes(command_data[offset+4..offset+8].try_into()?); let r = f32::from_le_bytes(command_data[offset+8..offset+12].try_into()?); let b = f32::from_le_bytes(command_data[offset+12..offset+16].try_into()?); let l = f32::from_le_bytes(command_data[offset+16..offset+20].try_into()?); s.set_inset(id, t, r, b, l); offset += 20; },
            18 => { let p = u32::from_le_bytes(command_data[offset..offset+4].try_into()?); let c = u32::from_le_bytes(command_data[offset+4..offset+8].try_into()?); s.add_child(p, c); offset += 8; },
            9 => { let id = u32::from_le_bytes(command_data[offset..offset+4].try_into()?); let v = u32::from_le_bytes(command_data[offset+4..offset+8].try_into()?); s.set_position(id, v); offset += 8; },
            _ => { offset = command_data.len(); }
        }
    }
    Ok(())
}

#[cfg(feature = "wasm3-support")]
pub fn process_commands(memory: &mut [u8], buffer_ptr: u32, state: &Arc<Mutex<SharedState>>) -> anyhow::Result<()> {
    let bs = buffer_ptr as usize; let clen = u32::from_le_bytes(memory[bs..bs+4].try_into()?);
    if clen == 0 { return Ok(()); }
    let _ = process_command_stream(state, &memory[bs+16 .. bs+16+clen as usize]);
    memory[bs..bs + 4].copy_from_slice(&0u32.to_le_bytes()); Ok(())
}

#[cfg(feature = "wasm3-support")]
pub fn sync_layout_to_wasm(memory: &mut [u8], buffer_ptr: u32, state: &SharedState) -> anyhow::Result<()> {
    let ls = buffer_ptr as usize + 16 + MAX_COMMAND_BYTES;
    for (&id, node) in &state.nodes {
        if id as usize >= MAX_NODES { continue; }
        if let Ok(layout) = state.taffy.layout(node.taffy_node) {
            let target = ls + (id as usize * 16);
            memory[target..target+4].copy_from_slice(&layout.location.x.to_le_bytes());
            memory[target+4..target+8].copy_from_slice(&layout.location.y.to_le_bytes());
            memory[target+8..target+12].copy_from_slice(&layout.size.width.to_le_bytes());
            memory[target+12..target+16].copy_from_slice(&layout.size.height.to_le_bytes());
        }
    }
    Ok(())
}

#[derive(uniffi::Object)] pub struct VelloHost { engine: Arc<Mutex<Option<EngineState>>>, active_surface_id: Mutex<Option<SurfaceId>>, surfaces: Mutex<HashMap<u64, SurfaceState>>, next_surface_id: AtomicU64 }
#[uniffi::export] impl VelloHost {
    #[uniffi::constructor] pub fn new() -> Arc<Self> { Arc::new(Self { engine: Arc::new(Mutex::new(None)), active_surface_id: Mutex::new(None), surfaces: Mutex::new(HashMap::new()), next_surface_id: AtomicU64::new(1) }) }
    pub fn init_native(&self, ptr: u64, ddir: String, w: u32, h: u32) {
        self.prepare_engine_sync(ddir);
        #[cfg(target_os = "android")] { let sh = Arc::new(SafeWindowHandle::new_android(ptr)); self.attach_surface_sync(vello::wgpu::SurfaceTarget::from(sh.clone()), w, h, Some(sh)); }
        #[cfg(target_os = "ios")] { let sh = Arc::new(SafeWindowHandle::new_ios(ptr)); self.attach_surface_sync(vello::wgpu::SurfaceTarget::from(sh.clone()), w, h, Some(sh)); }
    }
    pub fn resize_native(&self, width: u32, height: u32) { if let Some(id) = *self.active_surface_id.lock().unwrap() { let mut surfs = self.surfaces.lock().unwrap(); if let Some(s) = surfs.get_mut(&id.0) { if let Some(e) = &mut *self.engine.lock().unwrap() { e.context.resize_surface(&mut s.surface, width, height); render_frame(e, s); } } } }
    pub fn stop_native(&self) { if let Some(id) = self.active_surface_id.lock().unwrap().take() { self.surfaces.lock().unwrap().remove(&id.0); } }
    pub fn tick(&self) {
        if let Some(id) = *self.active_surface_id.lock().unwrap() {
            let mut eg = self.engine.lock().unwrap(); let mut surfs = self.surfaces.lock().unwrap();
            if let (Some(e), Some(s)) = (&mut *eg, surfs.get_mut(&id.0)) {
                #[cfg(feature = "wasm3-support")] {
                    let mem = unsafe { &mut *e._rt.memory_mut() };
                    let _ = process_commands(mem, e.shared_buffer_ptr, &e.shared_state);
                    let _ = e.tick_fn.call();
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
            if let Some(id) = hit { #[cfg(feature = "wasm3-support")] { let _ = e.on_click_fn.call(id); } }
        }
    }
    pub fn is_initialized(&self) -> bool { self.active_surface_id.lock().unwrap().is_some() }
    pub fn prepare_engine(&self, ddir: String) { self.prepare_engine_sync(ddir); }
}

impl VelloHost {
    fn prepare_engine_sync(&self, ddir: String) {
        let mut guard = self.engine.lock().unwrap(); if guard.is_some() { return; }
        if let Ok(e) = pollster::block_on(setup_engine(ddir, self.engine.clone())) { *guard = Some(e); }
    }
    fn attach_surface_sync(&self, target: vello::wgpu::SurfaceTarget<'static>, w: u32, h: u32, sh: Option<Arc<SafeWindowHandle>>) {
        let mut eg = self.engine.lock().unwrap(); let e = eg.as_mut().expect("Engine not prepared");
        if let Ok(surface) = pollster::block_on(e.context.create_surface(target, w, h, vello::wgpu::PresentMode::AutoVsync)) {
            let bl = e.context.devices[surface.dev_id].device.create_pipeline_layout(&vello::wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&e.blit_bind_group_layout], push_constant_ranges: &[] });
            let blit_p = e.context.devices[surface.dev_id].device.create_render_pipeline(&vello::wgpu::RenderPipelineDescriptor { label: None, layout: Some(&bl), vertex: vello::wgpu::VertexState { module: &e.blit_shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() }, fragment: Some(vello::wgpu::FragmentState { module: &e.blit_shader, entry_point: Some("fs_main"), targets: &[Some(vello::wgpu::ColorTargetState { format: surface.config.format, blend: Some(vello::wgpu::BlendState::REPLACE), write_mask: vello::wgpu::ColorWrites::ALL })], compilation_options: Default::default() }), primitive: vello::wgpu::PrimitiveState::default(), depth_stencil: None, multisample: vello::wgpu::MultisampleState::default(), multiview: None, cache: None });
            let nid = self.next_surface_id.fetch_add(1, Ordering::SeqCst);
            self.surfaces.lock().unwrap().insert(nid, SurfaceState { surface, blit_pipeline: blit_p, offscreen_texture: None, window_handle: sh });
            *self.active_surface_id.lock().unwrap() = Some(SurfaceId(nid));
        }
    }
    pub async fn init_with_target_rust(&self, target: vello::wgpu::SurfaceTarget<'static>, ddir: String, width: u32, height: u32) {
        #[cfg(not(target_arch = "wasm32"))] self.prepare_engine_sync(ddir);
        self.attach_surface_sync(target, width, height, None);
    }
    pub fn get_state(&self) -> std::sync::MutexGuard<'_, Option<EngineState>> { self.engine.lock().unwrap() }
    pub fn get_state_mut(&self) -> std::sync::MutexGuard<'_, Option<EngineState>> { self.engine.lock().unwrap() }
    pub fn apply_commands(&self, command_data: &[u8]) { if let Some(s) = &*self.engine.lock().unwrap() { let _ = process_command_stream(&s.shared_state, command_data); } }
}

async fn setup_engine(ddir: String, _es: Arc<Mutex<Option<EngineState>>>) -> anyhow::Result<EngineState> {
    let mut context = RenderContext::new(); let dev_id = context.device(None).await.unwrap();
    let dev = &context.devices[dev_id].device;
    let blit_shader = dev.create_shader_module(vello::wgpu::ShaderModuleDescriptor { label: None, source: vello::wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()) });
    let blit_bl = dev.create_bind_group_layout(&vello::wgpu::BindGroupLayoutDescriptor { label: None, entries: &[vello::wgpu::BindGroupLayoutEntry { binding: 0, visibility: vello::wgpu::ShaderStages::FRAGMENT, ty: vello::wgpu::BindingType::Texture { sample_type: vello::wgpu::TextureSampleType::Float { filterable: true }, view_dimension: vello::wgpu::TextureViewDimension::D2, multisampled: false }, count: None }, vello::wgpu::BindGroupLayoutEntry { binding: 1, visibility: vello::wgpu::ShaderStages::FRAGMENT, ty: vello::wgpu::BindingType::Sampler(vello::wgpu::SamplerBindingType::Filtering), count: None }] });
    let sampler = dev.create_sampler(&vello::wgpu::SamplerDescriptor { mag_filter: vello::wgpu::FilterMode::Linear, min_filter: vello::wgpu::FilterMode::Linear, ..Default::default() });
    let renderer = Renderer::new(dev, RendererOptions { antialiasing_support: vello::AaSupport::all(), pipeline_cache: None, num_init_threads: None, use_cpu: false }).unwrap();
    let state = Arc::new(Mutex::new(SharedState::new()));
    #[cfg(feature = "wasm3-support")] {
        let wasm_path = format!("{}/guest.wasm", ddir); let wasm = std::fs::read(&wasm_path).or_else(|_| std::fs::read("guest.wasm")).unwrap();
        let env = wasm3::Environment::new().unwrap(); let rt = env.create_runtime(1024 * 2048).unwrap();
        let mut module = rt.load_module(env.parse_module(wasm).unwrap()).unwrap();
        let bptr = module.find_function::<(), u32>("vello_get_shared_buffer_ptr").unwrap().call().unwrap();
        let s_inner = state.clone();
        let _ = module.link_closure("env", "ui_force_layout", move |ctx, ()| { let mem = unsafe { &mut *ctx.memory_mut() }; let _ = process_commands(mem, bptr, &s_inner); Ok(()) });
        let main_fn = module.find_function::<(), ()>("main").or_else(|_| module.find_function::<(), ()>("_main")).unwrap();
        let tick_fn = module.find_function::<(), ()>("guest_tick").or_else(|_| module.find_function::<(), ()>("_guest_tick")).unwrap();
        let on_click_fn = module.find_function::<(u32,), ()>("on_node_click").or_else(|_| module.find_function::<(u32,), ()>("_on_node_click")).unwrap();
        let _ = main_fn.call(); let memory = unsafe { &mut *rt.memory_mut() }; let _ = process_commands(memory, bptr, &state);
        Ok(EngineState { context, renderer, shared_state: state, tick_fn: unsafe { std::mem::transmute(tick_fn) }, on_click_fn: unsafe { std::mem::transmute(on_click_fn) }, _rt: rt, shared_buffer_ptr: bptr, blit_bind_group_layout: blit_bl, sampler, blit_shader })
    }
    #[cfg(not(feature = "wasm3-support"))] Ok(EngineState { context, renderer, shared_state: state, blit_bind_group_layout: blit_bl, sampler, blit_shader })
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
    e.renderer.render_to_texture(&e.context.devices[s.surface.dev_id].device, &e.context.devices[s.surface.dev_id].queue, &scene, &off_t.create_view(&Default::default()), &vello::RenderParams { base_color: Color::from_rgb8(0, 0, 255), width: w, height: h, antialiasing_method: vello::AaConfig::Area }).unwrap();
    if let Ok(st) = s.surface.surface.get_current_texture() {
        let mut enc = e.context.devices[s.surface.dev_id].device.create_command_encoder(&Default::default());
        { let mut rp = enc.begin_render_pass(&vello::wgpu::RenderPassDescriptor { label: None, color_attachments: &[Some(vello::wgpu::RenderPassColorAttachment { view: &st.texture.create_view(&Default::default()), resolve_target: None, ops: vello::wgpu::Operations { load: vello::wgpu::LoadOp::Clear(vello::wgpu::Color::TRANSPARENT), store: vello::wgpu::StoreOp::Store }, depth_slice: None })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None }); rp.set_pipeline(&s.blit_pipeline); rp.set_bind_group(0, blit_bg, &[]); rp.draw(0..3, 0..1); }
        e.context.devices[s.surface.dev_id].queue.submit(Some(enc.finish())); st.present();
    }
}
EOF
sh crates/host-mac/build.sh || true # 确保环境准备好
