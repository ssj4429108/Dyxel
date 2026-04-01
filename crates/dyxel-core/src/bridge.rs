// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT


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
use tokio::sync::Notify;

use crate::engine::{setup_engine, LogicState, RenderState};
use crate::platform::{SafeWindowHandle, SurfaceId};
use crate::renderer::render_frame;
use crate::state::SharedState;
use dyxel_render_api::{DeviceHandle, QueueHandle, SharedPtr, SharedMutex};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;

#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    // Legacy single-touch events (deprecated)
    TouchDown { x: f32, y: f32 },
    TouchMove { x: f32, y: f32 },
    TouchUp { x: f32, y: f32 },
    
    // New multi-touch events with Input Proxy
    PointerDown { pointer_id: u32, x: f32, y: f32, pressure: f32 },
    PointerMove { pointer_id: u32, x: f32, y: f32 },
    PointerUp { pointer_id: u32, x: f32, y: f32 },
    PointerCancel,
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
        target: Option<dyxel_render_api::SurfaceTargetHandle>,
        surface: Option<dyxel_render_api::SurfaceHandle>,
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
    TogglePerfOverlay,
    SetContinuousRender(bool),
}

// =============== Input Proxy with GestureArena ===============

#[cfg(not(target_arch = "wasm32"))]
use dyxel_gesture::{GestureRouter, GestureSettings, SpatialHitTester, GestureProvider, GestureType};
#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::get_handler_registry;
#[cfg(not(target_arch = "wasm32"))]
use crate::handler_registry::HandlerType;

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// Thread-local GestureRouter for the Logic Thread
    static GESTURE_ROUTER: std::cell::RefCell<Option<GestureRouter>> = std::cell::RefCell::new(None);
    /// Thread-local pointer to LogicState for gesture callbacks
    static LOGIC_STATE_PTR: std::cell::Cell<*const LogicState> = std::cell::Cell::new(std::ptr::null());
}

/// Gesture provider implementation that uses HandlerRegistry
#[cfg(not(target_arch = "wasm32"))]
struct HandlerRegistryGestureProvider;

#[cfg(not(target_arch = "wasm32"))]
impl GestureProvider for HandlerRegistryGestureProvider {
    fn get_node_gestures(&self, node_id: u32) -> Vec<GestureType> {
        let registry = get_handler_registry().lock().unwrap();
        let mut gestures = Vec::new();
        
        if registry.has_handler(node_id, HandlerType::Tap) {
            gestures.push(GestureType::Tap);
        }
        if registry.has_handler(node_id, HandlerType::DoubleTap) {
            gestures.push(GestureType::DoubleTap);
        }
        if registry.has_handler(node_id, HandlerType::LongPress) {
            gestures.push(GestureType::LongPress);
        }
        if registry.has_handler(node_id, HandlerType::Pan) {
            gestures.push(GestureType::Pan);
        }
        
        log::info!("get_node_gestures for node {}: {:?}", node_id, gestures);
        gestures
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_gesture_router_initialized(logic: &LogicState) {
    GESTURE_ROUTER.with(|router| {
        if router.borrow().is_none() {
            // Store LogicState pointer for callback access
            LOGIC_STATE_PTR.with(|ptr| ptr.set(logic as *const LogicState));
            
            // Get shared buffer pointer for hit testing
            let bptr = *logic.shared_buffer_ptr.lock().unwrap();
            
            // Create hit tester using shared buffer (with spatial index for O(1) hit testing)
            let hit_tester: Box<dyn dyxel_gesture::HitTester> = if let Some(bptr) = bptr {
                let mem = unsafe { &mut *logic._rt.memory_mut() };
                let shared_buffer_ptr = unsafe { 
                    mem.as_mut_ptr().add(bptr as usize) as *const dyxel_shared::SharedBuffer 
                };
                unsafe {
                    let mut tester = SpatialHitTester::new(shared_buffer_ptr);
                    tester.sync(); // Initial sync
                    log::info!("SpatialHitTester initialized");
                    Box::new(tester)
                }
            } else {
                Box::new(dyxel_gesture::NoOpHitTester)
            };

            // Get runtime reference for callback
            let rt_ptr = &logic._rt as *const wasm3::Runtime;
            let shared_buffer_ptr = bptr;

            // Create gesture router with callback that dispatches to WASM
            let settings = GestureSettings::default();
            let gesture_provider: Box<dyn GestureProvider> = Box::new(HandlerRegistryGestureProvider);
            let new_router = GestureRouter::new(
                settings,
                hit_tester,
                gesture_provider,
                move |event| {
                    dispatch_gesture_event(rt_ptr, shared_buffer_ptr, event);
                },
            );

            *router.borrow_mut() = Some(new_router);
            log::info!("GestureRouter initialized with direct gesture dispatch");
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn dispatch_gesture_event(
    rt_ptr: *const wasm3::Runtime,
    shared_buffer_ptr: Option<u32>,
    event: dyxel_gesture::GestureEvent,
) {
    use dyxel_shared::push_command;
    use dyxel_gesture::GestureEventType;
    use crate::handler_registry::HandlerType;

    if rt_ptr.is_null() || shared_buffer_ptr.is_none() {
        return;
    }

    let bptr = shared_buffer_ptr.unwrap();

    // Build bubble path from target to root using LogicState
    let bubble_path = LOGIC_STATE_PTR.with(|ptr| {
        let logic_ptr = ptr.get();
        if logic_ptr.is_null() {
            vec![event.target_node_id]
        } else {
            build_bubble_path(event.target_node_id, unsafe { &*logic_ptr })
        }
    });
    
    // Determine handler type from event type and tap_count
    let handler_type = match event.event_type {
        GestureEventType::Tap => {
            // Use tap_count to determine handler type
            match event.tap_count {
                1 => Some(HandlerType::Tap),
                2 => Some(HandlerType::DoubleTap),
                _ => Some(HandlerType::Tap), // For triple+ taps, fall back to Tap handler
            }
        }
        GestureEventType::LongPressStart | GestureEventType::LongPressEnd => Some(HandlerType::LongPress),
        GestureEventType::PanStart | GestureEventType::PanUpdate | GestureEventType::PanEnd => Some(HandlerType::Pan),
        _ => None,
    };
    
    // SAFETY: This is called from the Logic Thread where the runtime is valid
    unsafe {
        let mem = &mut *(*rt_ptr).memory_mut();
        let shared_buffer = &mut *(mem.as_mut_ptr().add(bptr as usize) as *mut dyxel_shared::SharedBuffer);

        if let Some(ht) = handler_type {
            // Use HandlerRegistry to find the actual handler target
            let registry = get_handler_registry().lock().unwrap();
            let handler_node = registry.find_handler(&bubble_path, ht);
            drop(registry);
            if let Some(handler_node) = handler_node {
                // Send direct gesture event (WASM should not bubble)
                match event.event_type {
                    GestureEventType::Tap => {
                        // Use tap_count to determine which command to send
                        match event.tap_count {
                            1 => {
                                push_command!(shared_buffer, DirectGestureTap, handler_node, event.x, event.y);
                                log::info!("DirectGesture: Tap on node {} (target was {}) at ({:.1},{:.1})", 
                                    handler_node, event.target_node_id, event.x, event.y);
                            }
                            2 => {
                                push_command!(shared_buffer, DirectGestureDoubleTap, handler_node, event.x, event.y);
                                log::info!("DirectGesture: DoubleTap on node {} (target was {}) at ({:.1},{:.1})", 
                                    handler_node, event.target_node_id, event.x, event.y);
                            }
                            _ => {
                                // For triple+ taps, send as Tap for now
                                push_command!(shared_buffer, DirectGestureTap, handler_node, event.x, event.y);
                                log::debug!("DirectGesture: Tap({}) on node {} (target was {}) at ({:.1},{:.1})", 
                                    event.tap_count, handler_node, event.target_node_id, event.x, event.y);
                            }
                        }
                    }
                    GestureEventType::LongPressStart => {
                        push_command!(shared_buffer, DirectGestureLongPress, handler_node, event.x, event.y);
                        log::info!("DirectGesture: LongPress on node {} (target was {}) at ({:.1},{:.1})", 
                            handler_node, event.target_node_id, event.x, event.y);
                    }
                    GestureEventType::PanStart => {
                        push_command!(shared_buffer, DirectGesturePanStart, handler_node, event.x, event.y);
                        log::info!("DirectGesture: PanStart on node {} (target was {}) at ({:.1},{:.1})", 
                            handler_node, event.target_node_id, event.x, event.y);
                    }
                    GestureEventType::PanUpdate => {
                        push_command!(shared_buffer, DirectGesturePanUpdate, handler_node, event.x, event.y, event.delta_x, event.delta_y);
                    }
                    GestureEventType::PanEnd => {
                        push_command!(shared_buffer, DirectGesturePanEnd, handler_node, event.x, event.y);
                        log::info!("DirectGesture: PanEnd on node {} (target was {}) at ({:.1},{:.1})", 
                            handler_node, event.target_node_id, event.x, event.y);
                    }
                    _ => {}
                }
                return; // Handled as direct gesture
            }
        }

        // Fallback to legacy gesture events (WASM will bubble)
        match event.event_type {
            GestureEventType::Tap => {
                // Use tap_count to determine which command to send
                match event.tap_count {
                    1 => { push_command!(shared_buffer, GestureTap, event.target_node_id, event.x, event.y); }
                    2 => { push_command!(shared_buffer, GestureDoubleTap, event.target_node_id, event.x, event.y); }
                    _ => { push_command!(shared_buffer, GestureTap, event.target_node_id, event.x, event.y); }
                }
            }
            GestureEventType::LongPressStart => {
                push_command!(shared_buffer, GestureLongPressStart, event.target_node_id, event.x, event.y);
            }
            GestureEventType::LongPressEnd => {
                push_command!(shared_buffer, GestureLongPressEnd, event.target_node_id, event.x, event.y);
            }
            GestureEventType::PanStart => {
                push_command!(shared_buffer, GesturePanStart, event.target_node_id, event.x, event.y);
            }
            GestureEventType::PanUpdate => {
                push_command!(shared_buffer, GesturePanUpdate, event.target_node_id, event.x, event.y, event.delta_x, event.delta_y);
            }
            GestureEventType::PanEnd => {
                push_command!(shared_buffer, GesturePanEnd, event.target_node_id, event.x, event.y, event.delta_x, event.delta_y);
            }
            _ => {
                log::debug!("Unhandled gesture event: {:?}", event.event_type);
            }
        }
    }
}

/// Build bubble path from target node to root
#[cfg(not(target_arch = "wasm32"))]
fn build_bubble_path(target_node: u32, logic: &LogicState) -> Vec<u32> {
    let path = vec![target_node];
    
    // Walk up parent chain using SharedState
    // This queries the Host-side tree structure
    if let Ok(_state) = logic.shared_state.try_lock() {
        let _current = target_node;
        // TODO: Traverse parent chain once parent pointers are available in SharedState
        // For now, just return the target node
        let _ = _current; // Suppress unused warning
    }
    
    path
}

#[cfg(not(target_arch = "wasm32"))]
fn process_input_internal(logic: &mut LogicState, event: InputEvent) {
    use dyxel_shared::{InputEventType, RawInputEvent};

    log::info!("DyxelInput: process_input_internal event={:?}", event);

    // Ensure GestureRouter is initialized
    ensure_gesture_router_initialized(logic);

    // Convert InputEvent to RawInputEvent
    let (event_type, pointer_id, x, y, pressure) = match event {
        InputEvent::TouchDown { x, y } => {
            (InputEventType::PointerDown, 0, x, y, 1.0)
        }
        InputEvent::TouchMove { x, y } => {
            (InputEventType::PointerMove, 0, x, y, 1.0)
        }
        InputEvent::TouchUp { x, y } => {
            (InputEventType::PointerUp, 0, x, y, 0.0)
        }
        InputEvent::PointerDown { pointer_id, x, y, pressure } => {
            (InputEventType::PointerDown, pointer_id, x, y, pressure)
        }
        InputEvent::PointerMove { pointer_id, x, y } => {
            (InputEventType::PointerMove, pointer_id, x, y, 1.0)
        }
        InputEvent::PointerUp { pointer_id, x, y } => {
            (InputEventType::PointerUp, pointer_id, x, y, 0.0)
        }
        InputEvent::PointerCancel => {
            (InputEventType::PointerCancel, 0, 0.0, 0.0, 0.0)
        }
    };

    // Get timestamp from host (microseconds)
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    // Create raw event
    let raw_event = RawInputEvent {
        timestamp,
        pointer_id,
        event_type: event_type as u8,
        _padding: [0; 3],
        x,
        y,
        pressure,
        delta_x: 0.0,
        delta_y: 0.0,
        target_node_id: 0,
        flags: 0,
    };

    // Route through GestureRouter
    GESTURE_ROUTER.with(|router| {
        if let Some(ref mut router) = *router.borrow_mut() {
            log::info!("DyxelInput: Routing through GestureRouter ptr={} type={:?}", raw_event.pointer_id, raw_event.event_type);
            router.sync(); // Sync spatial index before hit testing
            router.handle_input_event(&raw_event);
            log::info!("DyxelInput: GestureRouter processed event");
        } else {
            log::warn!("DyxelInput: GestureRouter not initialized!");
        }
    });
}

// =============== Platform-specific synchronization primitives ===============
// SharedPtr and SharedMutex are imported from dyxel_render_api

type EngineStatusMutex = StdMutex<EngineStatus>;
type EngineReadyNotify = Notify;

#[cfg(not(target_arch = "wasm32"))]
type SharedMutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;
#[cfg(target_arch = "wasm32")]
type SharedMutexGuard<'a, T> = std::cell::RefMut<'a, T>;

type AsyncGuard<'a, T> = std::sync::MutexGuard<'a, T>;

#[allow(dead_code)]
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
    // Opaque instance storage (wgpu::Instance for Vello backend)
    // Stored as Box<dyn Any> to avoid exposing wgpu types
    #[cfg(not(target_arch = "wasm32"))]
    instance: StdMutex<Option<Box<dyn std::any::Any + Send + Sync>>>,
    // Shared state - used directly in WASM builds, managed by threads in native builds
    shared_state: SharedPtr<SharedMutex<SharedState>>,
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
        #[cfg(not(target_arch = "wasm32"))]
        let (render_complete_tx, render_complete_rx) = mpsc::channel(); // VSync signal: Render -> Logic

        // Create shared state (used directly in WASM, managed by threads in native)
        let shared_state = SharedPtr::new(SharedMutex::new(crate::state::SharedState::new()));
        
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
            shared_state: shared_state.clone(),
        });

        #[cfg(not(target_arch = "wasm32"))]
        {
            let render_tx_for_logic = render_tx.clone();
            let render_complete_rx = render_complete_rx; // VSync signal receiver

            // 1. Logic Thread (Thinker)
            thread::Builder::new()
                .name("DyxelLogic".into())
                .spawn(move || {

                    let mut logic_opt: Option<LogicState> = None;
                    let mut lifecycle = Lifecycle::Stopped;

                    loop {
                        // Clear any pending VSync signals to prevent frame lag accumulation
                        // Logic Thread should sync with latest VSync, not old ones
                        while render_complete_rx.try_recv().is_ok() {}
                        
                        // Receive message (block when stopped/paused to save CPU)
                        let msg_res = if lifecycle == Lifecycle::Running {
                            // Running: non-blocking check then wait for VSync if no message
                            match logic_rx.try_recv() {
                                Ok(msg) => Ok(msg),
                                Err(std::sync::mpsc::TryRecvError::Empty) => {
                                    // No message, execute tick and sleep
                                    if let Some(ref mut l) = logic_opt {
                                        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
                                        {
                                            use crate::runtime::{process_commands, sync_layout_to_wasm, is_render_needed, clear_dirty_tracker};
                                            
                                            // Execute WASM tick (produces commands)
                                            if let Some(tick) = l.tick_fn.lock().unwrap().as_ref() {
                                                if let Err(e) = tick.call() {
                                                    log::error!("LogicThread: WASM tick failed: {}", e);
                                                }
                                            }
                                            
                                            // Debug: read counters
                                            if let (Ok(_get_events), Ok(_get_gestures), Ok(_get_clicks)) = (
                                                l._rt.find_function::<(), u32>("dyxel_get_event_count"),
                                                l._rt.find_function::<(), u32>("dyxel_get_gesture_count"),
                                                l._rt.find_function::<(), u32>("dyxel_get_click_count")
                                            ) {
                                                // WASM counters debug (removed)
                                            }
                                            
                                            // Process WASM commands
                                            let bptr = *l.shared_buffer_ptr.lock().unwrap();
                                            if let Some(bptr) = bptr {
                                                let mem = unsafe { &mut *l._rt.memory_mut() };
                                                let _ = process_commands(mem, bptr, &l.shared_state);
                                                
                                                // Sync layout results back to WASM
                                                let _ = sync_layout_to_wasm(
                                                    mem,
                                                    bptr,
                                                    &l.shared_state.lock().unwrap(),
                                                );
                                            }
                                            
                                            // Only trigger render if transaction completed and dirty nodes exist
                                            if is_render_needed() {
                                                let dirty_count = crate::runtime::get_dirty_tracker()
                                                    .map(|dt| dt.iter_dirty_nodes().count())
                                                    .unwrap_or(0);
                                                if dirty_count > 0 {

                                                }
                                                let _ = render_tx_for_logic.send(RenderMessage::RequestDraw);
                                                
                                                // VSync: Wait for render completion before next tick
                                                // This ensures Logic and Render are synchronized
                                                match render_complete_rx.recv_timeout(Duration::from_millis(33)) {
                                                    Ok(_) => {}, // Render completed, continue
                                                    Err(_) => {
                                                        // Timeout - render may be slow, continue anyway
                                                        log::warn!("LogicThread: Render timeout, continuing");
                                                    }
                                                }
                                                
                                                clear_dirty_tracker();
                                            } else {
                                                // No render needed, wait for next VSync signal from Render Thread
                                                // This keeps Logic Thread synchronized with display refresh
                                                let _ = render_complete_rx.recv_timeout(Duration::from_millis(33));
                                            }
                                        }
                                    }
                                    continue;
                                }
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                    log::error!("LogicThread: Channel disconnected");
                                    return;
                                }
                            }
                        } else {
                            // Stopped/Paused: block waiting for message
                            logic_rx.recv()
                        };

                        // Process received message
                        if let Ok(msg) = msg_res {
                            match msg {
                                LogicMessage::SetReady(l) => {
                                    logic_opt = Some(l);
                                    lifecycle = Lifecycle::Running;
                                }
                                LogicMessage::Input(event) => {
                                    if let Some(ref mut l) = logic_opt { process_input_internal(l, event); }
                                }
                                LogicMessage::LoadWasm(path) => {

                                    if let Some(ref mut l) = logic_opt { 
                                        if let Err(e) = l.load_wasm(path) {
                                            log::error!("LogicThread: LoadWasm failed: {}", e);
                                        }
                                    }
                                }
                                LogicMessage::Pause => {

                                    lifecycle = Lifecycle::Paused;
                                }
                                LogicMessage::Resume => {

                                    lifecycle = Lifecycle::Running;
                                }
                                LogicMessage::Shutdown => {

                                    return;
                                }
                            }
                        } else {
                            log::error!("LogicThread: Channel disconnected");
                            return;
                        }
                    }
                })
                .expect("Failed to spawn LogicThread");

            // 2. Render Thread (Rasterizer)
            let surfaces_ptr = surfaces.clone();
            let active_surface_ptr = active_surface_id.clone();
            let notify_ptr = engine_ready_notify.clone();
            let render_complete_tx = render_complete_tx.clone(); // VSync signal sender

            thread::Builder::new()
                .name("DyxelRender".into())
                .spawn(move || {

                    let mut render_opt: Option<RenderState> = None;
                    let mut lifecycle = Lifecycle::Stopped;
                    let mut continuous_render = true; // 默认开启连续渲染模式（最大性能）

                    loop {
                        // Process messages - either block or poll depending on mode
                        let mut latest_resize = None;
                        let mut draw_requested = continuous_render; // Continuous mode: always draw
                        let mut control_msgs = Vec::new();
                        
                        if continuous_render {
                            // Continuous mode: non-blocking poll for messages
                            while let Ok(msg) = render_rx.try_recv() {
                                match msg {
                                    RenderMessage::Resize { width, height } => {
                                        latest_resize = Some((width, height));
                                    }
                                    RenderMessage::RequestDraw => {
                                        // In continuous mode, ignore RequestDraw (we draw every loop)
                                    }
                                    RenderMessage::SetContinuousRender(enabled) => {
                                        continuous_render = enabled;
                                        draw_requested = enabled;

                                    }
                                    _ => {
                                        control_msgs.push(msg);
                                    }
                                }
                            }
                        } else {
                            // Event-driven mode: block on first message
                            let msg = match render_rx.recv() {
                                Ok(msg) => msg,
                                Err(_) => {
                                    log::error!("RenderThread: Channel disconnected, shutting down");
                                    return;
                                }
                            };
                            
                            // Process the first message
                            match msg {
                                RenderMessage::Resize { width, height } => {
                                    latest_resize = Some((width, height));
                                }
                                RenderMessage::RequestDraw => {
                                    draw_requested = true;
                                }
                                RenderMessage::SetContinuousRender(enabled) => {
                                    continuous_render = enabled;
                                    draw_requested = enabled;

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
                                    RenderMessage::SetContinuousRender(enabled) => {
                                        continuous_render = enabled;
                                        draw_requested = enabled;

                                    }
                                    _ => {
                                        control_msgs.push(next);
                                    }
                                }
                            }
                        }

                        // 1. Process all control messages in order (CreateSurface, Suspend, etc.)
                        for m in control_msgs {
                            match m {
                                RenderMessage::SetReady(r) => {

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

                                    *active_surface_ptr.lock_guard().unwrap() = Some(sid);
                                    lifecycle = Lifecycle::Running;
                                }
                                RenderMessage::Suspend(ack) => {
                                    lifecycle = Lifecycle::Stopped;
                                    if let Some(ref r) = render_opt {
                                        // Downcast context to get device and queue
                                        if let Some(v_ctx) = r.context.downcast_ref::<vello::util::RenderContext>() {
                                            let dev = &v_ctx.devices[0].device;
                                            let queue = &v_ctx.devices[0].queue;
                                            let dev_handle = DeviceHandle::new(dev);
                                            let queue_handle = QueueHandle::new(queue);
                                            r.backend.sync_gpu(dev_handle, queue_handle);
                                        }
                                    }
                                    let _ = ack.send(());
                                }
                                RenderMessage::Shutdown => {

                                    return;
                                }
                                RenderMessage::TogglePerfOverlay => {
                                    if let Some(ref r) = render_opt {
                                        r.enable_perf_overlay();

                                    }
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
                        } else if draw_requested && lifecycle == Lifecycle::Running {
                            let active_id = *active_surface_ptr.lock_guard().unwrap();
                            if let (Some(ref mut r), Some(id)) = (&mut render_opt, active_id) {
                                let mut surfs = surfaces_ptr.lock_guard().unwrap();
                                if let Some(s) = surfs.get_mut(&id.0) {
                                    if !continuous_render {
                                        log::trace!("RenderThread: Rendering frame for surface {:?}", id);
                                    }
                                    render_frame(r, s.as_mut());
                                    
                                    // Signal Logic Thread that render is complete (VSync)
                                    // This synchronizes Logic and Render threads
                                    // Frame rate is now determined by display VSync (60/120/144Hz)
                                    let _ = render_complete_tx.send(());
                                } else {
                                    log::warn!("RenderThread: Active surface {:?} not found in map", id);
                                }
                            } else {
                                log::trace!("RenderThread: Draw ignored (no active surface or no render_opt)");
                            }
                        }
                    }
                })
                .expect("Failed to spawn RenderThread");
        }
        host
    }

    pub async fn prepare_engine(&self, ddir: String) {

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
                    // Downcast RenderContext to get Vello's instance
                    if let Some(v_ctx) = render.context.downcast_ref::<vello::util::RenderContext>() {
                        *self.instance.lock().unwrap() = Some(Box::new(v_ctx.instance.clone()));
                    }

                    if let Some(tx) = &*self.logic_tx.lock().unwrap() {
                        let _ = tx.send(LogicMessage::SetReady(logic));
                    }
                    if let Some(tx) = &*self.render_tx.lock().unwrap() {
                        let _ = tx.send(RenderMessage::SetReady(render));
                    }
                }

                {

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

    // === Legacy single-touch API (deprecated, kept for compatibility) ===
    pub fn on_touch(&self, x: f32, y: f32) {
        self.on_pointer_down(0, x, y, 1.0);
    }

    // === New multi-touch Input Proxy API ===
    
    /// 指针按下（支持多指）
    pub fn on_pointer_down(&self, pointer_id: u32, x: f32, y: f32, pressure: f32) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            log::info!("DyxelInput: on_pointer_down pid={} x={:.1} y={:.1}", pointer_id, x, y);
            let _ = tx.send(LogicMessage::Input(InputEvent::PointerDown {
                pointer_id,
                x,
                y,
                pressure,
            }));
        }
    }

    /// 指针移动（支持多指）
    pub fn on_pointer_move(&self, pointer_id: u32, x: f32, y: f32) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            log::info!("DyxelInput: on_pointer_move pid={} x={:.1} y={:.1}", pointer_id, x, y);
            let _ = tx.send(LogicMessage::Input(InputEvent::PointerMove {
                pointer_id,
                x,
                y,
            }));
        }
    }

    /// 指针抬起（支持多指）
    pub fn on_pointer_up(&self, pointer_id: u32, x: f32, y: f32) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            log::info!("DyxelInput: on_pointer_up pid={} x={:.1} y={:.1}", pointer_id, x, y);
            let _ = tx.send(LogicMessage::Input(InputEvent::PointerUp {
                pointer_id,
                x,
                y,
            }));
        }
    }

    /// 指针取消（支持多指）
    pub fn on_pointer_cancel(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            log::info!("DyxelInput: on_pointer_cancel");
            let _ = tx.send(LogicMessage::Input(InputEvent::PointerCancel));
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
            // Create wgpu::SurfaceTarget and wrap it in SurfaceTargetHandle
            let wgpu_target: vello::wgpu::SurfaceTarget<'static> = sh.clone().into();
            let target_handle = dyxel_render_api::SurfaceTargetHandle::new(wgpu_target);
            self.setup(target_handle, _w, _h, Some(sh)).await;
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
            // Clear all surfaces, not just active one
            let mut surfaces = self.surfaces.lock_guard().unwrap();
            let count = surfaces.len();
            if count > 0 {

                surfaces.clear();
            }
            self.active_surface_id.lock_guard().unwrap().take();

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
    
    /// Toggle performance overlay display (FPS, Memory, CPU)
    pub fn toggle_perf_overlay(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.render_tx.lock().unwrap() {
            match tx.send(RenderMessage::TogglePerfOverlay) {
                Ok(_) => (),
                Err(e) => log::error!("toggle_perf_overlay: Failed to send: {:?}", e),
            }
        }
    }
}

impl DyxelHost {
    /// Get the shared state (used by web crate)
    /// 
    /// Returns Some(shared_state) on WASM builds, None on native builds
    /// (native builds use thread-local shared state)
    pub fn get_shared_state(&self) -> Option<SharedPtr<SharedMutex<SharedState>>> {
        // On WASM, return the shared state directly
        // On native, return None as state is managed by threads
        #[cfg(target_arch = "wasm32")]
        {
            Some(self.shared_state.clone())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = &self.shared_state; // Silence unused warning
            None
        }
    }
    
    /// Set continuous render mode (for performance testing)
    /// When enabled, render thread will render as fast as possible without waiting for RequestDraw
    pub fn set_continuous_render(&self, enabled: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.render_tx.lock().unwrap() {
            match tx.send(RenderMessage::SetContinuousRender(enabled)) {
                Ok(_) => (),
                Err(e) => log::error!("set_continuous_render: Failed to send: {:?}", e),
            }
        }
    }
    
    /// Setup a surface for rendering
    /// 
    /// The target should be a wgpu::SurfaceTarget<'static> wrapped in SurfaceTargetHandle
    /// (for Vello backend), or None for other backends
    pub async fn setup(
        &self,
        target: dyxel_render_api::SurfaceTargetHandle,
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

            self.engine_ready_notify.wait().await;
        } else {

        }

        let nid = self.next_surface_id.fetch_add(1, Ordering::SeqCst);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.render_tx.lock().unwrap() {


            // On macOS/Desktop, create surface on main thread to avoid Metal panic
            let (target, surface) = if cfg!(any(
                target_os = "macos",
                target_os = "windows",
                target_os = "linux"
            )) {
                let inst_lock = self.instance.lock().unwrap();
                if let Some(instance_any) = inst_lock.as_ref() {
                    // Downcast to wgpu::Instance for Vello backend
                    if let Some(instance) = instance_any.downcast_ref::<vello::wgpu::Instance>() {
                        // Try to downcast target to wgpu::SurfaceTarget
                        let mut target_opt: Option<dyxel_render_api::SurfaceTargetHandle> = Some(target);
                        if let Some(wgpu_target) = target_opt.take().unwrap().into_inner::<vello::wgpu::SurfaceTarget<'static>>() {
                            match instance.create_surface(wgpu_target) {
                                Ok(s) => {
                                    (None, Some(dyxel_render_api::SurfaceHandle::new(s)))
                                },
                                Err(e) => {
                                    log::error!("setup: Failed to create surface on main thread: {}", e);
                                    (None, None)
                                }
                            }
                        } else {
                            (target_opt, None)
                        }
                    } else {
                        (Some(target), None)
                    }
                } else {
                    (Some(target), None)
                }
            } else {
                (Some(target), None)
            };


            match tx.send(RenderMessage::CreateSurface {
                target,
                surface,
                width,
                height,
                nid,
            }) {
                Ok(_) => (),
                Err(e) => log::error!("setup: Failed to send CreateSurface: {:?}", e),
            }
            match tx.send(RenderMessage::RequestDraw) {
                Ok(_) => (),
                Err(e) => log::error!("setup: Failed to send RequestDraw: {:?}", e),
            }
        }

        // Resume LogicThread if it was paused (e.g., after Back button/activity restart)
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {

            let _ = tx.send(LogicMessage::Resume);
        }
    }
}
