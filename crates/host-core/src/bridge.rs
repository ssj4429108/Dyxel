use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use kurbo::Vec2;

use crate::platform::{SurfaceId, SurfaceState, SafeWindowHandle};
use crate::engine::{EngineState, setup_engine};
use crate::renderer::render_frame;
use crate::input::hit_test_recursive;

#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Object))]
pub struct VelloHost { 
    pub engine: Arc<Mutex<Option<EngineState>>>, 
    pub active_surface_id: Mutex<Option<SurfaceId>>, 
    pub surfaces: Mutex<HashMap<u64, SurfaceState>>, 
    pub next_surface_id: AtomicU64 
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
impl VelloHost {
    #[cfg_attr(not(target_arch = "wasm32"), uniffi::constructor)] 
    pub fn new() -> Arc<Self> { 
        Arc::new(Self { 
            engine: Arc::new(Mutex::new(None)), 
            active_surface_id: Mutex::new(None), 
            surfaces: Mutex::new(HashMap::new()), 
            next_surface_id: AtomicU64::new(1) 
        }) 
    }
    
    pub fn resize_native(&self, width: u32, height: u32) { 
        if let Some(id) = *self.active_surface_id.lock().unwrap() { 
            let mut surfs = self.surfaces.lock().unwrap(); 
            if let Some(s) = surfs.get_mut(&id.0) { 
                if let Some(e) = &mut *self.engine.lock().unwrap() { 
                    e.context.resize_surface(&mut s.surface, width, height); 
                    render_frame(e, s); 
                } 
            } 
        } 
    }
    
    pub fn stop_native(&self) { 
        if let Some(id) = self.active_surface_id.lock().unwrap().take() { 
            self.surfaces.lock().unwrap().remove(&id.0); 
        } 
    }
    
    pub fn tick(&self) {
        let active_id = { *self.active_surface_id.lock().unwrap() };
        if let Some(id) = active_id {
            let mut eg = self.engine.lock().unwrap();
            let mut surfs = self.surfaces.lock().unwrap();
            if let (Some(e), Some(s)) = (&mut *eg, surfs.get_mut(&id.0)) {
                #[cfg(feature = "wasm3-support")] {
                    use crate::runtime::process_commands;
                    // 1. Guest Logic
                    if let Err(err) = e.tick_fn.call() { 
                        log::error!("Wasm tick failed: {}", err); 
                    }
                    
                    // 2. Host Process Commands
                    {
                        let mem = unsafe { &mut *e._rt.memory_mut() };
                        if let Err(err) = process_commands(mem, e.shared_buffer_ptr, &e.shared_state) {
                            log::error!("Failed to process commands after logic: {}", err);
                        }
                    }
                }
                
                // 3. Host Layout & Render
                render_frame(e, s); 

                #[cfg(feature = "wasm3-support")] {
                    use crate::runtime::sync_layout_to_wasm;
                    // 4. Host Sync Layout
                    let mem = unsafe { &mut *e._rt.memory_mut() };
                    if let Err(err) = sync_layout_to_wasm(mem, e.shared_buffer_ptr, &e.shared_state.lock().unwrap()) {
                        log::error!("Failed to sync layout results: {}", err);
                    }
                }
            }
        }
    }
    
    pub fn on_touch(&self, x: f32, y: f32) {
        if let Some(e) = &*self.engine.lock().unwrap() {
            let mp = Vec2::new(x as f64, y as f64);
            let hit = { 
                let sg = e.shared_state.lock().unwrap(); 
                sg.root_id.and_then(|rid| hit_test_recursive(rid, mp, &sg.nodes, &sg.taffy, Vec2::ZERO, &sg.click_listeners)) 
            };
            if let Some(_target_id) = hit { 
                #[cfg(feature = "wasm3-support")] { 
                    if let Err(err) = e.on_click_fn.call(_target_id) { 
                        log::error!("Wasm click failed: {}", err); 
                    }
                } 
            }
        }
    }
    
    pub fn is_initialized(&self) -> bool { 
        self.active_surface_id.lock().unwrap().is_some() 
    }
    
    pub async fn prepare_engine(&self, ddir: String) {
        if self.engine.lock().unwrap().is_some() { return; }
        if let Ok(e) = setup_engine(ddir, self.engine.clone()).await {
            let mut guard = self.engine.lock().unwrap();
            if guard.is_none() { *guard = Some(e); }
        }
    }
    
    pub async fn init_native(&self, surface_ptr: u64, ddir: String, w: u32, h: u32) {
        #[cfg(target_os = "android")] 
        let sh = Arc::new(SafeWindowHandle::new_android(surface_ptr));
        #[cfg(target_os = "ios")] 
        let sh = Arc::new(SafeWindowHandle::new_ios(surface_ptr));
        
        #[cfg(any(target_os = "android", target_os = "ios"))] 
        self.setup(vello::wgpu::SurfaceTarget::from(sh.clone()), ddir, w, h, Some(sh)).await;
        
        #[cfg(not(any(target_os = "android", target_os = "ios")))] 
        { let _ = (surface_ptr, ddir, w, h); }
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
                let bl = dev.create_pipeline_layout(&vello::wgpu::PipelineLayoutDescriptor { 
                    label: None, 
                    bind_group_layouts: &[&e.blit_bind_group_layout], 
                    push_constant_ranges: &[] 
                });
                let blit_p = dev.create_render_pipeline(&vello::wgpu::RenderPipelineDescriptor { 
                    label: None, 
                    layout: Some(&bl), 
                    vertex: vello::wgpu::VertexState { 
                        module: &e.blit_shader, 
                        entry_point: Some("vs_main"), 
                        buffers: &[], 
                        compilation_options: Default::default() 
                    }, 
                    fragment: Some(vello::wgpu::FragmentState { 
                        module: &e.blit_shader, 
                        entry_point: Some("fs_main"), 
                        targets: &[Some(vello::wgpu::ColorTargetState { 
                            format: surface.config.format, 
                            blend: Some(vello::wgpu::BlendState::REPLACE), 
                            write_mask: vello::wgpu::ColorWrites::ALL 
                        })], 
                        compilation_options: Default::default() 
                    }), 
                    primitive: vello::wgpu::PrimitiveState::default(), 
                    depth_stencil: None, 
                    multisample: vello::wgpu::MultisampleState::default(), 
                    multiview: None, 
                    cache: None 
                });
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
    
    pub fn get_state(&self) -> std::sync::MutexGuard<'_, Option<EngineState>> { 
        self.engine.lock().unwrap() 
    }
    
    pub fn get_state_mut(&self) -> std::sync::MutexGuard<'_, Option<EngineState>> { 
        self.engine.lock().unwrap() 
    }
    
    pub fn apply_commands(&self, command_data: &[u8]) { 
        if let Some(s) = &*self.engine.lock().unwrap() { 
            use crate::runtime::process_command_stream;
            let _ = process_command_stream(&s.shared_state, command_data); 
        } 
    }
}
