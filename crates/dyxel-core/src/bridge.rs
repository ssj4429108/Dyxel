// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#[cfg(not(target_arch = "wasm32"))]
use kurbo::Vec2;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex as StdMutex;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::engine::{setup_engine, LogicState, RenderState};
#[cfg(not(target_arch = "wasm32"))]
use crate::input::hit_test_recursive;
use crate::platform::{SafeWindowHandle, SurfaceId};
use crate::renderer::render_frame;
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
    Running,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Running,
    Paused,
    Stopped,
}

#[cfg(not(target_arch = "wasm32"))]
pub enum LogicMessage {
    SetReady(LogicState),
    Input(InputEvent),
    LoadWasm(String),
    Pause,
    Resume,
    Shutdown,
}

#[cfg(not(target_arch = "wasm32"))]
pub enum RenderMessage {
    SetReady(RenderState),
    CreateSurface {
        target: Option<vello::wgpu::SurfaceTarget<'static>>,
        surface: Option<vello::wgpu::Surface<'static>>,
        width: u32,
        height: u32,
        nid: u64,
    },
    SetSurfaceActive(SurfaceId),
    Resize {
        width: u32,
        height: u32,
    },
    RequestDraw,
    Suspend(mpsc::Sender<()>), // Sync barrier with ACK
    Shutdown,
}

#[cfg(not(target_arch = "wasm32"))]
fn process_input_internal(logic: &mut LogicState, event: InputEvent) {
    match event {
        InputEvent::TouchDown { x, y } => {
            let mp = Vec2::new(x as f64, y as f64);
            let hit = {
                let sg = logic.shared_state.lock().unwrap();
                sg.root_id.and_then(|rid| {
                    hit_test_recursive(
                        rid,
                        mp,
                        &sg.nodes,
                        &sg.taffy,
                        Vec2::ZERO,
                        &sg.click_listeners,
                    )
                })
            };
            if let Some(_target_id) = hit {
                #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
                {
                    if let Some(on_click) = logic.on_click_fn.lock().unwrap().as_ref() {
                        let _ = on_click.call(_target_id);
                    }
                }
            }
        }
        _ => {}
    }
}

// =============== Platform-specific synchronization primitives ===============

#[cfg(not(target_arch = "wasm32"))]
use crate::engine::{SharedMutex, SharedPtr};
#[cfg(target_arch = "wasm32")]
use crate::engine::{SharedMutex, SharedPtr};

type EngineStatusMutex = StdMutex<EngineStatus>;
type EngineReadyNotify = Notify;

#[cfg(not(target_arch = "wasm32"))]
type SharedMutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;
#[cfg(target_arch = "wasm32")]
type SharedMutexGuard<'a, T> = std::cell::RefMut<'a, T>;

type AsyncGuard<'a, T> = std::sync::MutexGuard<'a, T>;

trait SharedMutexExt<T> {
    fn lock_guard(&self) -> Result<SharedMutexGuard<'_, T>, ()>;
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

trait AsyncMutexExt<T: ?Sized> {
    fn lock_sync<'a>(&'a self) -> AsyncGuard<'a, T>
    where
        T: 'a;
}

impl<T: ?Sized> AsyncMutexExt<T> for StdMutex<T> {
    fn lock_sync<'a>(&'a self) -> AsyncGuard<'a, T>
    where
        T: 'a,
    {
        self.lock().unwrap()
    }
}

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
    logic_tx: StdMutex<Option<mpsc::Sender<LogicMessage>>>,
    #[cfg(not(target_arch = "wasm32"))]
    render_tx: StdMutex<Option<mpsc::Sender<RenderMessage>>>,

    engine_status: SharedPtr<EngineStatusMutex>,
    engine_ready_notify: SharedPtr<EngineReadyNotify>,
    pub active_surface_id: SharedPtr<SharedMutex<Option<SurfaceId>>>,
    pub next_surface_id: SharedPtr<AtomicU64>,
    pub surfaces: SharedPtr<SharedMutex<HashMap<u64, Box<dyn dyxel_render_api::SurfaceState>>>>,
    #[cfg(not(target_arch = "wasm32"))]
    pub first_frame_rendered: std::sync::atomic::AtomicBool,
    #[cfg(not(target_arch = "wasm32"))]
    instance: StdMutex<Option<vello::wgpu::Instance>>,
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
impl DyxelHost {
    #[cfg_attr(not(target_arch = "wasm32"), uniffi::constructor)]
    pub fn new() -> SharedPtr<Self> {
        let engine_status = SharedPtr::new(StdMutex::new(EngineStatus::Uninitialized));
        let engine_ready_notify = SharedPtr::new(EngineReadyNotify::new());
        let surfaces = SharedPtr::new(SharedMutex::new(HashMap::new()));
        let active_surface_id = SharedPtr::new(SharedMutex::new(None));
        let next_surface_id = SharedPtr::new(AtomicU64::new(1));

        #[cfg(not(target_arch = "wasm32"))]
        let (logic_tx, logic_rx) = mpsc::channel();
        #[cfg(not(target_arch = "wasm32"))]
        let (render_tx, render_rx) = mpsc::channel();

        let host = SharedPtr::new(Self {
            #[cfg(not(target_arch = "wasm32"))]
            logic_tx: StdMutex::new(Some(logic_tx)),
            #[cfg(not(target_arch = "wasm32"))]
            render_tx: StdMutex::new(Some(render_tx.clone())),
            engine_status: engine_status.clone(),
            engine_ready_notify: engine_ready_notify.clone(),
            active_surface_id: active_surface_id.clone(),
            next_surface_id: next_surface_id.clone(),
            surfaces: surfaces.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            first_frame_rendered: std::sync::atomic::AtomicBool::new(false),
            #[cfg(not(target_arch = "wasm32"))]
            instance: StdMutex::new(None),
        });

        #[cfg(not(target_arch = "wasm32"))]
        {
            let render_tx_for_logic = render_tx.clone();

            // 1. Logic Thread (Thinker)
            thread::Builder::new()
                .name("DyxelLogic".into())
                .spawn(move || {
                    log::info!("LogicThread: Thread spawned");
                    let mut logic_opt: Option<LogicState> = None;
                    let mut lifecycle = Lifecycle::Stopped;

                    loop {
                        // Receive message
                        let msg_res = if lifecycle == Lifecycle::Running {
                            logic_rx.try_recv().map_err(|e| anyhow::anyhow!(e))
                        } else {
                            logic_rx.recv().map_err(|e| anyhow::anyhow!(e))
                        };

                        if let Ok(msg) = msg_res {
                            match msg {
                                LogicMessage::SetReady(l) => {
                                    log::info!("LogicThread: Received SetReady, setting lifecycle to Running");
                                    logic_opt = Some(l);
                                    lifecycle = Lifecycle::Running;
                                }
                                LogicMessage::Input(event) => {
                                    log::debug!("LogicThread: Received Input {:?}", event);
                                    if let Some(ref mut l) = logic_opt { process_input_internal(l, event); }
                                }
                                LogicMessage::LoadWasm(path) => {
                                    log::info!("LogicThread: Received LoadWasm from {}", path);
                                    if let Some(ref mut l) = logic_opt { let _ = l.load_wasm(path); }
                                }
                                LogicMessage::Pause => {
                                    log::info!("LogicThread: Received Pause, setting lifecycle to Paused");
                                    lifecycle = Lifecycle::Paused;
                                }
                                LogicMessage::Resume => {
                                    log::info!("LogicThread: Received Resume, setting lifecycle to Running");
                                    lifecycle = Lifecycle::Running;
                                }
                                LogicMessage::Shutdown => {
                                    log::info!("LogicThread: Shutting down");
                                    return;
                                }
                            }
                        }

                        if lifecycle == Lifecycle::Running {
                            if let Some(ref mut l) = logic_opt {
                                #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
                                {
                                    use crate::runtime::{process_commands, sync_layout_to_wasm};
                                    if let Some(tick) = l.tick_fn.lock().unwrap().as_ref() {
                                        if let Err(e) = tick.call() {
                                            log::error!("LogicThread: WASM tick failed: {}", e);
                                        }
                                    }
                                    let bptr = *l.shared_buffer_ptr.lock().unwrap();
                                    if let Some(bptr) = bptr {
                                        let mem = unsafe { &mut *l._rt.memory_mut() };
                                        let _ = process_commands(mem, bptr, &l.shared_state);
                                        let _ = sync_layout_to_wasm(
                                            mem,
                                            bptr,
                                            &l.shared_state.lock().unwrap(),
                                        );
                                    }
                                }
                                let _ = render_tx_for_logic.send(RenderMessage::RequestDraw);
                            }
                            // thread::sleep(Duration::from_millis(16)); // ~60fps logic tick
                        }
                    }
                })
                .expect("Failed to spawn LogicThread");

            // 2. Render Thread (Rasterizer)
            let surfaces_ptr = surfaces.clone();
            let active_surface_ptr = active_surface_id.clone();
            let notify_ptr = engine_ready_notify.clone();

            thread::Builder::new()
                .name("DyxelRender".into())
                .spawn(move || {
                    log::info!("RenderThread: Thread spawned");
                    let mut render_opt: Option<RenderState> = None;
                    let mut lifecycle = Lifecycle::Stopped;

                    loop {
                        // Block on first message
                        let msg = render_rx.recv().unwrap();

                        // Coalesce messages
                        let mut latest_resize = None;
                        let mut draw_requested = false;
                        let mut control_msgs = Vec::new();

                        // Process the first message
                        match msg {
                            RenderMessage::Resize { width, height } => {
                                latest_resize = Some((width, height));
                            }
                            RenderMessage::RequestDraw => {
                                draw_requested = true;
                            }
                            _ => {
                                control_msgs.push(msg);
                            }
                        }

                        // Drain the rest of the queue
                        while let Ok(next) = render_rx.try_recv() {
                            match next {
                                RenderMessage::Resize { width, height } => {
                                    latest_resize = Some((width, height));
                                }
                                RenderMessage::RequestDraw => {
                                    draw_requested = true;
                                }
                                _ => {
                                    control_msgs.push(next);
                                }
                            }
                        }

                        // 1. Process all control messages in order (CreateSurface, Suspend, etc.)
                        for m in control_msgs {
                            match m {
                                RenderMessage::SetReady(r) => {
                                    log::info!("RenderThread: Received SetReady, setting lifecycle to Running");
                                    render_opt = Some(r);
                                    lifecycle = Lifecycle::Running;
                                    notify_ptr.notify();
                                }
                                RenderMessage::CreateSurface {
                                    target,
                                    surface,
                                    width,
                                    height,
                                    nid,
                                } => {
                                    log::info!(
                                        "RenderThread: Creating surface id: {}, size: {}x{}, render_opt is_none: {}",
                                        nid,
                                        width,
                                        height,
                                        render_opt.is_none()
                                    );
                                    if let Some(ref mut r) = render_opt {
                                        match r.backend.create_surface_state(
                                            &mut r.context,
                                            target,
                                            surface,
                                            0,
                                            width,
                                            height,
                                        ) {
                                            Ok(ss) => {
                                                log::info!(
                                                    "RenderThread: Surface created successfully"
                                                );
                                                surfaces_ptr.lock_guard().unwrap().insert(nid, ss);
                                                *active_surface_ptr.lock_guard().unwrap() =
                                                    Some(SurfaceId(nid));
                                                lifecycle = Lifecycle::Running;
                                            }
                                            Err(e) => log::error!(
                                                "RenderThread: Failed to create surface: {}",
                                                e
                                            ),
                                        }
                                    }
                                }
                                RenderMessage::SetSurfaceActive(sid) => {
                                    log::info!("RenderThread: Setting active surface: {:?}", sid);
                                    *active_surface_ptr.lock_guard().unwrap() = Some(sid);
                                    lifecycle = Lifecycle::Running;
                                }
                                RenderMessage::Suspend(ack) => {
                                    log::info!("RenderThread: Suspending GPU, setting lifecycle to Stopped");
                                    lifecycle = Lifecycle::Stopped;
                                    if let Some(ref r) = render_opt {
                                        let dev = &r.context.devices[0].device;
                                        let queue = &r.context.devices[0].queue;
                                        r.backend.sync_gpu(dev, queue);
                                    }
                                    let _ = ack.send(());
                                }
                                RenderMessage::Shutdown => {
                                    log::info!("RenderThread: Shutting down");
                                    return;
                                }
                                _ => {}
                            }
                        }

                        // 2. Handle coalesced Resize/RequestDraw
                        if let Some((width, height)) = latest_resize {
                            let active_id = *active_surface_ptr.lock_guard().unwrap();
                            log::debug!(
                                "RenderThread: Coalesced Resize to {}x{}, active_id: {:?}",
                                width,
                                height,
                                active_id
                            );
                            if let (Some(ref mut r), Some(id)) = (&mut render_opt, active_id) {
                                let mut surfs = surfaces_ptr.lock_guard().unwrap();
                                if let Some(s) = surfs.get_mut(&id.0) {
                                    s.resize(&mut r.context, width, height);
                                    render_frame(r, s.as_mut());
                                }
                            }
                        } else if draw_requested {
                            let active_id = *active_surface_ptr.lock_guard().unwrap();
                            if let (Some(ref mut r), Some(id)) = (&mut render_opt, active_id) {
                                if lifecycle == Lifecycle::Stopped {
                                    log::info!("RenderThread: Auto-resuming from RequestDraw");
                                    lifecycle = Lifecycle::Running;
                                }

                                if lifecycle == Lifecycle::Running {
                                    let mut surfs = surfaces_ptr.lock_guard().unwrap();
                                    if let Some(s) = surfs.get_mut(&id.0) {
                                        log::trace!("RenderThread: Rendering frame for surface {:?}", id);
                                        render_frame(r, s.as_mut());
                                    } else {
                                        log::warn!("RenderThread: Active surface {:?} not found in map", id);
                                    }
                                }
                            } else {
                                log::trace!("RenderThread: RequestDraw ignored (no active surface or no render_opt)");
                            }
                        }
                    }
                })
                .expect("Failed to spawn RenderThread");
        }
        host
    }

    pub async fn prepare_engine(&self, ddir: String) {
        log::info!("prepare_engine: START - ddir={}", ddir);
        {
            let mut status = self.engine_status.lock_sync();
            if !matches!(*status, EngineStatus::Uninitialized) {
                return;
            }
            *status = EngineStatus::Loading;
        }

        match setup_engine(ddir).await {
            Ok((logic, render)) => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // Store instance for main-thread surface creation
                    *self.instance.lock().unwrap() = Some(render.context.instance.clone());

                    if let Some(tx) = &*self.logic_tx.lock().unwrap() {
                        let _ = tx.send(LogicMessage::SetReady(logic));
                    }
                    if let Some(tx) = &*self.render_tx.lock().unwrap() {
                        let _ = tx.send(RenderMessage::SetReady(render));
                    }
                }

                {
                    log::info!("prepare_engine: Setting status to Running");
                    let mut status = self.engine_status.lock_sync();
                    *status = EngineStatus::Running;
                    self.engine_ready_notify.notify();
                }
            }
            Err(e) => {
                log::error!("DyxelHost: Engine setup failed: {}", e);
                let mut status = self.engine_status.lock_sync();
                *status = EngineStatus::Error(e.to_string());
                self.engine_ready_notify.notify();
            }
        }
    }

    pub async fn load_wasm(&self, wasm_path: String) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            let _ = tx.send(LogicMessage::LoadWasm(wasm_path));
        }
    }

    pub fn on_touch(&self, x: f32, y: f32) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            let _ = tx.send(LogicMessage::Input(InputEvent::TouchDown { x, y }));
        }
    }

    pub fn resize_native(&self, width: u32, height: u32) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.render_tx.lock().unwrap() {
            let _ = tx.send(RenderMessage::Resize { width, height });
        }
    }

    pub async fn init_native(&self, _surface_ptr: u64, ddir: String, _w: u32, _h: u32) {
        let needs_prepare = {
            let status = self.engine_status.lock_sync();
            matches!(*status, EngineStatus::Uninitialized)
        };

        if needs_prepare {
            self.prepare_engine(ddir.clone()).await;
        }

        #[cfg(target_os = "android")]
        {
            let sh = SharedPtr::new(SafeWindowHandle::new_android(_surface_ptr));
            self.setup(
                vello::wgpu::SurfaceTarget::from(sh.clone()),
                _w,
                _h,
                Some(sh),
            )
            .await;
        }
    }

    pub fn stop_native(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (ack_tx, ack_rx) = mpsc::channel();
            if let Some(tx) = &*self.render_tx.lock().unwrap() {
                let _ = tx.send(RenderMessage::Suspend(ack_tx));
                let _ = ack_rx.recv_timeout(Duration::from_millis(500)); // Barrier
            }
            if let Some(tx) = &*self.logic_tx.lock().unwrap() {
                let _ = tx.send(LogicMessage::Pause);
            }
            if let Some(id) = self.active_surface_id.lock_guard().unwrap().take() {
                self.surfaces.lock_guard().unwrap().remove(&id.0);
            }
        }
    }

    pub fn shutdown(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(tx) = &*self.logic_tx.lock().unwrap() {
                let _ = tx.send(LogicMessage::Shutdown);
            }
            if let Some(tx) = &*self.render_tx.lock().unwrap() {
                let _ = tx.send(RenderMessage::Shutdown);
            }
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(*self.engine_status.lock_sync(), EngineStatus::Running)
    }

    pub fn is_engine_ready(&self) -> bool {
        self.is_ready()
    }

    pub fn is_initialized(&self) -> bool {
        self.active_surface_id.lock_guard().unwrap().is_some()
    }

    pub fn tick(&self) {
        // No-op for now, logic runs in its own thread
    }
}

impl DyxelHost {
    pub async fn setup(
        &self,
        target: vello::wgpu::SurfaceTarget<'static>,
        width: u32,
        height: u32,
        _handle: Option<SharedPtr<SafeWindowHandle>>,
    ) {
        // Fix: Ensure lock is dropped before await to keep Future Send
        let already_running = {
            let status = self.engine_status.lock_sync();
            matches!(*status, EngineStatus::Running)
        };

        if !already_running {
            log::info!("setup: Waiting for engine ready notify...");
            self.engine_ready_notify.wait().await;
        } else {
            log::info!("setup: Engine already running, proceeding");
        }

        let nid = self.next_surface_id.fetch_add(1, Ordering::SeqCst);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.render_tx.lock().unwrap() {
            log::info!("setup: Creating surface on main thread if necessary");

            // On macOS/Desktop, create surface on main thread to avoid Metal panic
            let (target, surface) = if cfg!(any(
                target_os = "macos",
                target_os = "windows",
                target_os = "linux"
            )) {
                let inst_lock = self.instance.lock().unwrap();
                if let Some(instance) = inst_lock.as_ref() {
                    log::info!("setup: Creating wgpu::Surface on main thread");
                    match instance.create_surface(target) {
                        Ok(s) => (None, Some(s)),
                        Err(e) => {
                            log::error!("setup: Failed to create surface on main thread: {}", e);
                            (None, None)
                        }
                    }
                } else {
                    (Some(target), None)
                }
            } else {
                (Some(target), None)
            };

            log::info!("setup: Sending CreateSurface message");
            match tx.send(RenderMessage::CreateSurface {
                target,
                surface,
                width,
                height,
                nid,
            }) {
                Ok(_) => log::info!("setup: CreateSurface message sent successfully"),
                Err(e) => log::error!("setup: Failed to send CreateSurface: {:?}", e),
            }
            match tx.send(RenderMessage::RequestDraw) {
                Ok(_) => log::info!("setup: RequestDraw message sent successfully"),
                Err(e) => log::error!("setup: Failed to send RequestDraw: {:?}", e),
            }
        }

        // Resume LogicThread if it was paused (e.g., after Back button/activity restart)
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            log::info!("setup: Sending Resume to LogicThread");
            let _ = tx.send(LogicMessage::Resume);
        }
    }
}
