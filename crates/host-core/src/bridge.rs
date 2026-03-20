use std::collections::HashMap;
use std::sync::{Arc, mpsc, atomic::{AtomicU64, Ordering}};
use kurbo::Vec2;
use tokio::sync::{Mutex as AsyncMutex, Notify};
use std::sync::Mutex as StdMutex;
use std::thread;
use std::time::Duration;

use crate::platform::{SurfaceId, SurfaceState, SafeWindowHandle};
use crate::engine::{EngineState, setup_engine};
use crate::renderer::render_frame;
use crate::input::hit_test_recursive;

#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    TouchDown { x: f32, y: f32 },
    TouchMove { x: f32, y: f32 },
    TouchUp { x: f32, y: f32 },
}

pub enum EngineStatus {
    Uninitialized,
    Loading,
    Ready(EngineState),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Running,
    Paused,
    Stopped,
}

enum EngineMessage {
    SetReady(EngineState),
    SetSurfaceActive(SurfaceId),
    Resize { width: u32, height: u32 },
    Input(InputEvent),
    Suspend,
    Shutdown,
}

fn process_input_internal(e: &mut EngineState, event: InputEvent) {
    match event {
        InputEvent::TouchDown { x, y } => {
            let mp = Vec2::new(x as f64, y as f64);
            let hit = { 
                let sg = e.shared_state.lock().unwrap(); 
                sg.root_id.and_then(|rid| hit_test_recursive(rid, mp, &sg.nodes, &sg.taffy, Vec2::ZERO, &sg.click_listeners)) 
            };
            if let Some(_target_id) = hit { 
                #[cfg(feature = "wasm3-support")] { 
                    let _ = e.on_click_fn.call(_target_id);
                } 
            }
        }
        _ => {}
    }
}

#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Object))]
pub struct VelloHost { 
    #[cfg(not(target_arch = "wasm32"))]
    command_tx: StdMutex<Option<mpsc::Sender<EngineMessage>>>,
    engine_status: Arc<AsyncMutex<EngineStatus>>,
    engine_ready_notify: Arc<Notify>,
    pub active_surface_id: Arc<StdMutex<Option<SurfaceId>>>, 
    pub next_surface_id: Arc<AtomicU64>,
    pub surfaces: Arc<StdMutex<HashMap<u64, SurfaceState>>>,
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
impl VelloHost {
    #[cfg_attr(not(target_arch = "wasm32"), uniffi::constructor)] 
    pub fn new() -> Arc<Self> { 
        let engine_status = Arc::new(AsyncMutex::new(EngineStatus::Uninitialized));
        let engine_ready_notify = Arc::new(Notify::new());
        let surfaces = Arc::new(StdMutex::new(HashMap::new()));
        let active_surface_id = Arc::new(StdMutex::new(None));
        let next_surface_id = Arc::new(AtomicU64::new(1));

        #[cfg(not(target_arch = "wasm32"))]
        let (tx, rx) = mpsc::channel();

        let host = Arc::new(Self { 
            #[cfg(not(target_arch = "wasm32"))]
            command_tx: StdMutex::new(Some(tx)),
            engine_status: engine_status.clone(), 
            engine_ready_notify: engine_ready_notify.clone(),
            active_surface_id: active_surface_id.clone(), 
            next_surface_id: next_surface_id.clone(),
            surfaces: surfaces.clone(),
        });

        #[cfg(not(target_arch = "wasm32"))]
        {
            let status_ptr = engine_status.clone();
            let surfaces_ptr = surfaces.clone();
            let active_surface_ptr = active_surface_id.clone();

            thread::Builder::new()
                .name("UIMainThread".to_string())
                .stack_size(8 * 1024 * 1024) 
                .spawn(move || {
                log::info!("UIMainThread: Autonomous loop active");
                let mut input_queue = Vec::new();
                let mut lifecycle = Lifecycle::Stopped;

                let handle_msg = |msg: EngineMessage, lc: &mut Lifecycle, inputs: &mut Vec<InputEvent>| -> bool {
                    match msg {
                        EngineMessage::SetReady(engine) => {
                            let mut status = pollster::block_on(status_ptr.lock());
                            *status = EngineStatus::Ready(engine);
                            *lc = Lifecycle::Running;
                        }
                        EngineMessage::SetSurfaceActive(sid) => {
                            *active_surface_ptr.lock().unwrap() = Some(sid);
                            *lc = Lifecycle::Running;
                        }
                        EngineMessage::Resize { width, height } => {
                            let active_id = *active_surface_ptr.lock().unwrap();
                            if let Some(id) = active_id {
                                let mut status = pollster::block_on(status_ptr.lock());
                                let mut surfs = surfaces_ptr.lock().unwrap();
                                if let (EngineStatus::Ready(ref mut e), Some(s)) = (&mut *status, surfs.get_mut(&id.0)) {
                                    e.context.resize_surface(&mut s.surface, width, height);
                                    render_frame(e, s);
                                }
                            }
                        }
                        EngineMessage::Input(event) => { inputs.push(event); }
                        EngineMessage::Suspend => { *lc = Lifecycle::Stopped; }
                        EngineMessage::Shutdown => { return true; }
                    }
                    false
                };

                loop {
                    while let Ok(msg) = rx.try_recv() {
                        if handle_msg(msg, &mut lifecycle, &mut input_queue) { return; }
                    }

                    if lifecycle == Lifecycle::Running {
                        let active_id = *active_surface_ptr.lock().unwrap();
                        if let Some(id) = active_id {
                            let mut status = pollster::block_on(status_ptr.lock());
                            let mut surfs = surfaces_ptr.lock().unwrap();
                            if let (EngineStatus::Ready(ref mut e), Some(s)) = (&mut *status, surfs.get_mut(&id.0)) {
                                for event in input_queue.drain(..) { process_input_internal(e, event); }
                                #[cfg(feature = "wasm3-support")] {
                                    use crate::runtime::process_commands;
                                    let _ = e.tick_fn.call();
                                    let mem = unsafe { &mut *e._rt.memory_mut() };
                                    let _ = process_commands(mem, e.shared_buffer_ptr, &e.shared_state);
                                }
                                render_frame(e, s);
                                #[cfg(feature = "wasm3-support")] {
                                    use crate::runtime::sync_layout_to_wasm;
                                    let mem = unsafe { &mut *e._rt.memory_mut() };
                                    let _ = sync_layout_to_wasm(mem, e.shared_buffer_ptr, &e.shared_state.lock().unwrap());
                                }
                            }
                        }
                        thread::sleep(Duration::from_millis(1));
                    } else {
                        if let Ok(msg) = rx.recv() {
                            if handle_msg(msg, &mut lifecycle, &mut input_queue) { return; }
                        }
                    }
                }
            }).expect("Failed to spawn UIMainThread");
        }
        host
    }

    pub async fn prepare_engine_async(&self, ddir: String) {
        {
            let mut status = self.engine_status.lock().await;
            if !matches!(*status, EngineStatus::Uninitialized) { return; }
            *status = EngineStatus::Loading;
        }
        let result = setup_engine(ddir, Arc::new(StdMutex::new(None))).await;
        match result {
            Ok(engine) => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(tx) = &*self.command_tx.lock().unwrap() { let _ = tx.send(EngineMessage::SetReady(engine)); }
                #[cfg(target_arch = "wasm32")]
                { *self.engine_status.lock().await = EngineStatus::Ready(engine); }
                self.engine_ready_notify.notify_waiters();
            }
            Err(e) => { 
                let mut status = self.engine_status.lock().await;
                *status = EngineStatus::Error(e.to_string()); 
                self.engine_ready_notify.notify_waiters();
            }
        }
    }

    pub fn tick(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            let mut status_lock = self.engine_status.blocking_lock();
            let mut surfs = self.surfaces.lock().unwrap();
            let active_id = *self.active_surface_id.lock().unwrap();
            if let (EngineStatus::Ready(ref mut e), Some(id)) = (&mut *status_lock, active_id) {
                if let Some(s) = surfs.get_mut(&id.0) {
                    #[cfg(feature = "wasm3-support")] {
                        use crate::runtime::process_commands;
                        let _ = e.tick_fn.call();
                        let mem = unsafe { &mut *e._rt.memory_mut() };
                        let _ = process_commands(mem, e.shared_buffer_ptr, &e.shared_state);
                    }
                    render_frame(e, s);
                }
            }
        }
    }

    pub fn on_touch(&self, x: f32, y: f32) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.command_tx.lock().unwrap() { let _ = tx.send(EngineMessage::Input(InputEvent::TouchDown { x, y })); }
        
        #[cfg(target_arch = "wasm32")]
        {
            let mut status_lock = self.engine_status.blocking_lock();
            if let EngineStatus::Ready(ref mut e) = *status_lock {
                process_input_internal(e, InputEvent::TouchDown { x, y });
            }
        }
    }

    pub fn resize_native(&self, width: u32, height: u32) { 
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.command_tx.lock().unwrap() { let _ = tx.send(EngineMessage::Resize { width, height }); }
        
        #[cfg(target_arch = "wasm32")]
        {
            let mut status_lock = self.engine_status.blocking_lock();
            let mut surfs = self.surfaces.lock().unwrap();
            let active_id = *self.active_surface_id.lock().unwrap();
            if let (EngineStatus::Ready(ref mut e), Some(id)) = (&mut *status_lock, active_id) {
                if let Some(s) = surfs.get_mut(&id.0) {
                    e.context.resize_surface(&mut s.surface, width, height);
                    render_frame(e, s);
                }
            }
        }
    }

    pub fn is_initialized(&self) -> bool { 
        let status = self.engine_status.blocking_lock();
        matches!(*status, EngineStatus::Ready(_)) && self.active_surface_id.lock().unwrap().is_some()
    }

    pub fn is_engine_ready(&self) -> bool {
        let status = self.engine_status.blocking_lock();
        matches!(*status, EngineStatus::Ready(_))
    }

    pub async fn init_native(&self, _surface_ptr: u64, ddir: String, _w: u32, _h: u32) {
        self.prepare_engine_async(ddir.clone()).await;
        #[cfg(target_os = "android")] let sh = Arc::new(SafeWindowHandle::new_android(_surface_ptr));
        #[cfg(target_os = "ios")] let sh = Arc::new(SafeWindowHandle::new_ios(_surface_ptr));
        #[cfg(any(target_os = "android", target_os = "ios"))] self.setup(vello::wgpu::SurfaceTarget::from(sh.clone()), _w, _h, Some(sh)).await;
    }

    pub fn stop_native(&self) { 
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.command_tx.lock().unwrap() { let _ = tx.send(EngineMessage::Suspend); }
        if let Some(id) = self.active_surface_id.lock().unwrap().take() { self.surfaces.lock().unwrap().remove(&id.0); } 
    }

    pub fn shutdown(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.command_tx.lock().unwrap() { let _ = tx.send(EngineMessage::Shutdown); }
    }
}

impl VelloHost {
    pub async fn setup(&self, target: vello::wgpu::SurfaceTarget<'static>, width: u32, height: u32, handle: Option<Arc<SafeWindowHandle>>) {
        loop {
            {
                let status = self.engine_status.lock().await;
                match *status {
                    EngineStatus::Ready(_) => break,
                    EngineStatus::Error(_) => return,
                    _ => {}
                }
            }
            #[cfg(not(target_arch = "wasm32"))] self.engine_ready_notify.notified().await;
            #[cfg(target_arch = "wasm32")] gloo_timers::future::TimeoutFuture::new(10).await;
        }

        let mut status_lock = self.engine_status.lock().await;
        if let EngineStatus::Ready(ref mut e) = *status_lock {
            if let Ok(surface) = e.context.create_surface(target, width, height, vello::wgpu::PresentMode::AutoVsync).await {
                let dev = &e.context.devices[surface.dev_id].device;
                let bl = dev.create_pipeline_layout(&vello::wgpu::PipelineLayoutDescriptor { 
                    label: None, bind_group_layouts: &[&e.blit_bind_group_layout], push_constant_ranges: &[] 
                });
                let blit_p = dev.create_render_pipeline(&vello::wgpu::RenderPipelineDescriptor { 
                    label: None, layout: Some(&bl), 
                    vertex: vello::wgpu::VertexState { module: &e.blit_shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() }, 
                    fragment: Some(vello::wgpu::FragmentState { 
                        module: &e.blit_shader, entry_point: Some("fs_main"), 
                        targets: &[Some(vello::wgpu::ColorTargetState { 
                            format: surface.config.format, blend: Some(vello::wgpu::BlendState::REPLACE), write_mask: vello::wgpu::ColorWrites::ALL 
                        })], 
                        compilation_options: Default::default() 
                    }), 
                    primitive: vello::wgpu::PrimitiveState::default(), depth_stencil: None, multisample: vello::wgpu::MultisampleState::default(), multiview: None, cache: None 
                });
                let nid = self.next_surface_id.fetch_add(1, Ordering::SeqCst);
                let mut ss = SurfaceState { surface, blit_pipeline: blit_p, offscreen_texture: None, window_handle: handle };
                render_frame(e, &mut ss);
                
                self.surfaces.lock().unwrap().insert(nid, ss);
                
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(tx) = &*self.command_tx.lock().unwrap() {
                    let _ = tx.send(EngineMessage::SetSurfaceActive(SurfaceId(nid)));
                }
                
                #[cfg(target_arch = "wasm32")]
                { *self.active_surface_id.lock().unwrap() = Some(SurfaceId(nid)); }
            }
        }
    }

    pub fn get_shared_state(&self) -> Option<Arc<StdMutex<crate::state::SharedState>>> {
        let status = self.engine_status.blocking_lock();
        if let EngineStatus::Ready(ref e) = *status { Some(e.shared_state.clone()) } else { None }
    }
}
