// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
#[cfg(not(target_arch = "wasm32"))]
use kurbo::Vec2;
use tokio::sync::{Mutex as AsyncMutex, Notify};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex as StdMutex;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use crate::platform::{SurfaceId, SafeWindowHandle};
use crate::engine::{EngineState, setup_engine};
use crate::renderer::render_frame;
#[cfg(not(target_arch = "wasm32"))]
use crate::input::hit_test_recursive;
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;

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

#[allow(dead_code)]
enum EngineMessage {
    SetReady(EngineState),
    SetSurfaceActive(SurfaceId),
    Resize { width: u32, height: u32 },
    Input(InputEvent),
    Suspend,
    Shutdown,
}

#[cfg(not(target_arch = "wasm32"))]
fn process_input_internal(e: &mut EngineState, event: InputEvent) {
    match event {
        InputEvent::TouchDown { x, y } => {
            let mp = Vec2::new(x as f64, y as f64);
            let hit = { 
                let sg = e.shared_state.lock().unwrap(); 
                sg.root_id.and_then(|rid| hit_test_recursive(rid, mp, &sg.nodes, &sg.taffy, Vec2::ZERO, &sg.click_listeners)) 
            };
            if let Some(_target_id) = hit { 
                #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] { 
                    if let Some(on_click) = e.on_click_fn.lock().unwrap().as_ref() {
                        let _ = on_click.call(_target_id);
                    }
                } 
            }
        }
        _ => {}
    }
}

// =============== Platform-specific synchronization primitives ===============

// Import SharedPtr and SharedMutex from engine
#[cfg(not(target_arch = "wasm32"))]
use crate::engine::{SharedPtr, SharedMutex};
#[cfg(target_arch = "wasm32")]
use crate::engine::{SharedPtr, SharedMutex};

// Async mutex for engine_status
type EngineStatusMutex = AsyncMutex<EngineStatus>;

// Notify for synchronization
type EngineReadyNotify = Notify;

// Guard types
#[cfg(not(target_arch = "wasm32"))]
type SharedMutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;
#[cfg(target_arch = "wasm32")]
type SharedMutexGuard<'a, T> = std::cell::RefMut<'a, T>;

// Async guard
type AsyncGuard<'a, T> = tokio::sync::MutexGuard<'a, T>;

// Trait extensions for cross-platform mutex operations
trait SharedMutexExt<T> {
    #[allow(dead_code)]
    fn lock_guard(&self) -> Result<SharedMutexGuard<'_, T>, ()>;
    #[allow(dead_code)]
    fn try_lock_guard(&self) -> Option<SharedMutexGuard<'_, T>>;
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> SharedMutexExt<T> for SharedMutex<T> {
    fn lock_guard(&self) -> Result<SharedMutexGuard<'_, T>, ()> {
        self.lock().map_err(|_| ())
    }
    fn try_lock_guard(&self) -> Option<SharedMutexGuard<'_, T>> {
        self.try_lock().ok()
    }
}

#[cfg(target_arch = "wasm32")]
impl<T> SharedMutexExt<T> for SharedMutex<T> {
    fn lock_guard(&self) -> Result<SharedMutexGuard<'_, T>, ()> {
        self.try_borrow_mut().map_err(|_| ())
    }
    fn try_lock_guard(&self) -> Option<SharedMutexGuard<'_, T>> {
        self.try_borrow_mut().ok()
    }
}

// Async mutex extensions
trait AsyncMutexExt<T: ?Sized> {
    async fn async_lock<'a>(&'a self) -> AsyncGuard<'a, T> where T: 'a;
    #[allow(dead_code)]
    fn try_async_lock(&self) -> Option<AsyncGuard<'_, T>>;
    #[allow(dead_code)]
    fn blocking_lock_guard(&self) -> AsyncGuard<'_, T>;
}

impl<T: ?Sized> AsyncMutexExt<T> for AsyncMutex<T> {
    async fn async_lock<'a>(&'a self) -> AsyncGuard<'a, T> where T: 'a {
        self.lock().await
    }
    fn try_async_lock(&self) -> Option<AsyncGuard<'_, T>> {
        self.try_lock().ok()
    }
    fn blocking_lock_guard(&self) -> AsyncGuard<'_, T> {
        self.blocking_lock()
    }
}

// Notify extensions
trait NotifyExt {
    async fn wait(&self);
    fn notify(&self);
}

impl NotifyExt for Notify {
    async fn wait(&self) {
        self.notified().await;
    }
    fn notify(&self) {
        self.notify_waiters();
    }
}

// =============== DyxelHost ===============

#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Object))]
pub struct DyxelHost { 
    #[cfg(not(target_arch = "wasm32"))]
    command_tx: StdMutex<Option<mpsc::Sender<EngineMessage>>>,
    engine_status: SharedPtr<EngineStatusMutex>,
    engine_ready_notify: SharedPtr<EngineReadyNotify>,
    pub active_surface_id: SharedPtr<SharedMutex<Option<SurfaceId>>>, 
    pub next_surface_id: SharedPtr<AtomicU64>,
    pub surfaces: SharedPtr<SharedMutex<HashMap<u64, Box<dyn dyxel_render_api::SurfaceState>>>>,
    #[cfg(not(target_arch = "wasm32"))]
    pub first_frame_rendered: std::sync::atomic::AtomicBool,
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
impl DyxelHost {
    #[cfg_attr(not(target_arch = "wasm32"), uniffi::constructor)] 
    pub fn new() -> SharedPtr<Self> { 
        let engine_status = SharedPtr::new(EngineStatusMutex::new(EngineStatus::Uninitialized));
        let engine_ready_notify = SharedPtr::new(EngineReadyNotify::new());
        let surfaces = SharedPtr::new(SharedMutex::new(HashMap::new()));
        let active_surface_id = SharedPtr::new(SharedMutex::new(None));
        let next_surface_id = SharedPtr::new(AtomicU64::new(1));

        #[cfg(not(target_arch = "wasm32"))]
        let (tx, rx) = mpsc::channel();

        let host = SharedPtr::new(Self { 
            #[cfg(not(target_arch = "wasm32"))]
            command_tx: StdMutex::new(Some(tx)),
            engine_status: engine_status.clone(), 
            engine_ready_notify: engine_ready_notify.clone(),
            active_surface_id: active_surface_id.clone(), 
            next_surface_id: next_surface_id.clone(),
            surfaces: surfaces.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            first_frame_rendered: std::sync::atomic::AtomicBool::new(false),
        });

        #[cfg(not(target_arch = "wasm32"))]
        {
            let status_ptr = engine_status.clone();
            let surfaces_ptr = surfaces.clone();
            let active_surface_ptr = active_surface_id.clone();
            let _host_ptr = SharedPtr::downgrade(&host);
            let notify_ptr = engine_ready_notify.clone();

            thread::Builder::new()
                .name("UIMainThread".to_string())
                .stack_size(8 * 1024 * 1024) 
                .spawn(move || {
                let mut input_queue = Vec::new();
                let mut lifecycle = Lifecycle::Stopped;

                let handle_msg = |msg: EngineMessage, lc: &mut Lifecycle, inputs: &mut Vec<InputEvent>| -> bool {
                    match msg {
                        EngineMessage::SetReady(engine) => {
                            let mut status = pollster::block_on(status_ptr.async_lock());
                            *status = EngineStatus::Ready(engine);
                            *lc = Lifecycle::Running;
                            notify_ptr.notify();
                        }
                        EngineMessage::SetSurfaceActive(sid) => {
                            *active_surface_ptr.lock_guard().unwrap() = Some(sid);
                            *lc = Lifecycle::Running;
                        }
                        EngineMessage::Resize { width, height } => {
                            let active_id = *active_surface_ptr.lock_guard().unwrap();
                            if let Some(id) = active_id {
                                let mut status = pollster::block_on(status_ptr.async_lock());
                                let mut surfs = surfaces_ptr.lock_guard().unwrap();
                                if let (EngineStatus::Ready(ref mut e), Some(s)) = (&mut *status, surfs.get_mut(&id.0)) {
                                    s.resize(&mut e.context, width, height);
                                    render_frame(e, s.as_mut());
                                }
                            }
                        }
                        EngineMessage::Input(event) => { inputs.push(event); }
                        EngineMessage::Suspend => { *lc = Lifecycle::Stopped; }
                        EngineMessage::Shutdown => { 
                            let status = pollster::block_on(status_ptr.async_lock());
                            if let EngineStatus::Ready(ref e) = *status {
                                e.on_lifecycle_event(dyxel_render_api::LifecycleEvent::Shutdown);
                            }
                            return true; 
                        }
                    }
                    false
                };

                loop {
                    while let Ok(msg) = rx.try_recv() {
                        if handle_msg(msg, &mut lifecycle, &mut input_queue) { return; }
                    }

                    if lifecycle == Lifecycle::Running {
                        let active_id = *active_surface_ptr.lock_guard().unwrap();
                        if let Some(id) = active_id {
                            let mut status = pollster::block_on(status_ptr.async_lock());
                            let mut surfs = surfaces_ptr.lock_guard().unwrap();
                            if let (EngineStatus::Ready(ref mut e), Some(s)) = (&mut *status, surfs.get_mut(&id.0)) {
                                for event in input_queue.drain(..) { process_input_internal(e, event); }
                                
                                #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] {
                                    use crate::runtime::{process_commands, sync_layout_to_wasm};
                                    if let Some(tick) = e.tick_fn.lock().unwrap().as_ref() {
                                        let _ = tick.call();
                                    }
                                    let bptr = *e.shared_buffer_ptr.lock().unwrap();
                                    if let Some(bptr) = bptr {
                                        let mem = unsafe { &mut *e._rt.memory_mut() };
                                        let _ = process_commands(mem, bptr, &e.shared_state);
                                        render_frame(e, s.as_mut());
                                        let _ = sync_layout_to_wasm(mem, bptr, &e.shared_state.lock().unwrap());
                                    } else {
                                        render_frame(e, s.as_mut());
                                    }
                                }

                                #[cfg(any(not(feature = "wasm3-support"), target_arch = "wasm32"))]
                                {
                                    render_frame(e, s.as_mut());
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

    pub async fn prepare_engine(&self, ddir: String) {
        self.prepare_engine_async(ddir, vec![]).await;
    }

    pub async fn prepare_engine_async(&self, ddir: String, _wasm_bytes: Vec<u8>) {
        {
            let mut status = self.engine_status.async_lock().await;
            if !matches!(*status, EngineStatus::Uninitialized) { 
                return; 
            }
            *status = EngineStatus::Loading;
        }
        use crate::engine::SharedPtr as EngineSharedPtr;
        use crate::engine::SharedMutex as EngineSharedMutex;
        let result = setup_engine(ddir, EngineSharedPtr::new(EngineSharedMutex::new(None))).await;
        match result {
            Ok(engine) => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(tx) = &*self.command_tx.lock().unwrap() { 
                    let _ = tx.send(EngineMessage::SetReady(engine)); 
                }
                #[cfg(target_arch = "wasm32")]
                { 
                    *self.engine_status.async_lock().await = EngineStatus::Ready(engine); 
                }
                self.engine_ready_notify.notify();
            }
            Err(e) => { 
                log::error!("DyxelHost: Engine setup failed: {}", e);
                let mut status = self.engine_status.async_lock().await;
                *status = EngineStatus::Error(e.to_string()); 
                self.engine_ready_notify.notify();
            }
        }
    }

    pub async fn load_wasm(&self, wasm_path: String) {
        loop {
            let n = self.engine_ready_notify.wait();
            
            {
                let status = self.engine_status.async_lock().await;
                match *status {
                    EngineStatus::Ready(_) => {
                        break;
                    }
                    EngineStatus::Error(ref e) => {
                        log::error!("DyxelHost: Cannot load WASM: Engine is in error state: {}", e);
                        return;
                    }
                    _ => {}
                }
            }
            n.await;
        }

        let status_lock = self.engine_status.async_lock().await;
        if let EngineStatus::Ready(ref e) = *status_lock {
            #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] {
                if let Err(err) = e.load_wasm(wasm_path) {
                    log::error!("DyxelHost: Failed to load WASM: {}", err);
                }
            }
            #[cfg(any(not(feature = "wasm3-support"), target_arch = "wasm32"))] {
                let _ = wasm_path;
                let _ = e;
                log::warn!("DyxelHost: load_wasm called but wasm3-support is not enabled or on WASM platform");
            }
        }
    }

    pub fn tick(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            if let (Some(mut status_lock), Some(mut surfs), Some(active_id)) = 
                (self.engine_status.try_async_lock(), self.surfaces.try_lock_guard(), self.active_surface_id.try_lock_guard()) {
                if let (EngineStatus::Ready(ref mut e), Some(id)) = (&mut *status_lock, *active_id) {
                    if let Some(s) = surfs.get_mut(&id.0) {
                        render_frame(e, s.as_mut());
                    }
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn on_touch(&self, x: f32, _y: f32) {
        if let Some(tx) = &*self.command_tx.lock().unwrap() { let _ = tx.send(EngineMessage::Input(InputEvent::TouchDown { x, y: _y })); }
    }

    pub fn resize_native(&self, width: u32, height: u32) { 
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.command_tx.lock().unwrap() { 
            let _ = tx.send(EngineMessage::Resize { width, height }); 
        }
        
        #[cfg(target_arch = "wasm32")]
        {
            // Directly handle resize on WASM platform
            use std::ops::DerefMut;
            if let Some(mut status) = self.engine_status.try_async_lock() {
                if let EngineStatus::Ready(ref mut e) = *status {
                    if let Some(active_id_guard) = self.active_surface_id.try_lock_guard() {
                        if let Some(active_id) = active_id_guard.as_ref() {
                            if let Some(mut surfs) = self.surfaces.try_lock_guard() {
                                if let Some(surface) = surfs.get_mut(&active_id.0) {
                                    surface.resize(&mut e.context, width, height);
                                    // Trigger a re-render
                                    render_frame(e, surface.deref_mut());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn is_initialized(&self) -> bool { 
        #[cfg(not(target_arch = "wasm32"))]
        {
            let status = self.engine_status.blocking_lock_guard();
            matches!(*status, EngineStatus::Ready(_)) && self.active_surface_id.lock_guard().unwrap().is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let (Some(status), Some(active)) = (self.engine_status.try_async_lock(), self.active_surface_id.try_lock_guard()) {
                matches!(*status, EngineStatus::Ready(_)) && active.is_some()
            } else {
                false
            }
        }
    }

    pub fn is_engine_ready(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let status = self.engine_status.blocking_lock_guard();
            matches!(*status, EngineStatus::Ready(_))
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(status) = self.engine_status.try_async_lock() {
                matches!(*status, EngineStatus::Ready(_))
            } else {
                false
            }
        }
    }

    pub async fn init_native(&self, _surface_ptr: u64, ddir: String, _w: u32, _h: u32) {
        self.prepare_engine(ddir.clone()).await;
        #[cfg(target_os = "android")] let sh = SharedPtr::new(SafeWindowHandle::new_android(_surface_ptr));
        #[cfg(target_os = "ios")] let sh = SharedPtr::new(SafeWindowHandle::new_ios(_surface_ptr));
        #[cfg(any(target_os = "android", target_os = "ios"))] self.setup(vello::wgpu::SurfaceTarget::from(sh.clone()), _w, _h, Some(sh)).await;
    }

    pub fn stop_native(&self) { 
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(tx) = &*self.command_tx.lock().unwrap() { let _ = tx.send(EngineMessage::Suspend); }
            // Remove surface from active_surface_id and surfaces, will re-initialize on next return
            if let Some(id) = self.active_surface_id.lock_guard().unwrap().take() { 
                self.surfaces.lock_guard().unwrap().remove(&id.0); 
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let (Some(mut id), Some(mut surfs)) = (self.active_surface_id.try_lock_guard(), self.surfaces.try_lock_guard()) {
                if let Some(sid) = id.take() {
                    surfs.remove(&sid.0);
                }
            }
        }
    }

    pub fn shutdown(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.command_tx.lock().unwrap() { let _ = tx.send(EngineMessage::Shutdown); }
    }
}

impl DyxelHost {
    pub async fn setup(&self, target: vello::wgpu::SurfaceTarget<'static>, width: u32, height: u32, _handle: Option<SharedPtr<SafeWindowHandle>>) {
        loop {
            let n = self.engine_ready_notify.wait();

            {
                let status = self.engine_status.async_lock().await;
                match *status {
                    EngineStatus::Ready(_) => {
                        break;
                    }
                    EngineStatus::Error(_) => {
                        log::error!("DyxelHost: setup engine is in Error state, aborting");
                        return;
                    }
                    _ => {}
                }
            }
            n.await;
        }

        let mut status_lock = self.engine_status.async_lock().await;
        if let EngineStatus::Ready(ref mut e) = *status_lock {
            let nid = self.next_surface_id.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut ss) = e.backend.create_surface_state(&mut e.context, Some(target), 0, width, height) {
                render_frame(e, ss.as_mut());
                e.on_lifecycle_event(dyxel_render_api::LifecycleEvent::FirstFrameDone);

                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.surfaces.lock_guard().unwrap().insert(nid, ss);                
                    if let Some(tx) = &*self.command_tx.lock().unwrap() {
                        let _ = tx.send(EngineMessage::SetSurfaceActive(SurfaceId(nid)));
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    if let Some(mut surfs) = self.surfaces.try_lock_guard() {
                        surfs.insert(nid, ss);
                    }
                    if let Some(mut id) = self.active_surface_id.try_lock_guard() {
                        *id = Some(SurfaceId(nid));
                    }
                }
            } else {
                log::error!("DyxelHost: Failed to create surface state");
            }
        }
    }

    pub fn get_shared_state(&self) -> Option<SharedPtr<SharedMutex<crate::state::SharedState>>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let status = self.engine_status.blocking_lock_guard();
            if let EngineStatus::Ready(ref e) = *status { Some(e.shared_state.clone()) } else { None }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(status) = self.engine_status.try_async_lock() {
                if let EngineStatus::Ready(ref e) = *status { Some(e.shared_state.clone()) } else { None }
            } else {
                None
            }
        }
    }
}
