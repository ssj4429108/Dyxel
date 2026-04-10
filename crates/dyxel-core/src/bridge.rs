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
use crate::text_input::sync_to_renderer;
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use dyxel_render_api::{DeviceHandle, QueueHandle, SharedMutex, SharedPtr};

#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    // Legacy single-touch events (deprecated)
    TouchDown {
        x: f32,
        y: f32,
    },
    TouchMove {
        x: f32,
        y: f32,
    },
    TouchUp {
        x: f32,
        y: f32,
    },

    // New multi-touch events with Input Proxy
    PointerDown {
        pointer_id: u32,
        x: f32,
        y: f32,
        pressure: f32,
    },
    PointerMove {
        pointer_id: u32,
        x: f32,
        y: f32,
    },
    PointerUp {
        pointer_id: u32,
        x: f32,
        y: f32,
    },
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
    /// Text input from keyboard (handled on main thread, sent to logic thread)
    TextInput { node_id: u32, text: String },
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
use crate::handler_registry::{HandlerRegistry, HandlerType};
#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::get_handler_registry;
#[cfg(not(target_arch = "wasm32"))]
use dyxel_gesture::{GestureConfig as V2GestureConfig, GestureType as V2GestureType};
#[cfg(not(target_arch = "wasm32"))]
use dyxel_gesture::{GestureRouter, HitTester, SpatialHitTester};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// Thread-local GestureRouter for the Logic Thread
    static GESTURE_ROUTER: std::cell::RefCell<Option<GestureRouter>> = std::cell::RefCell::new(None);
    /// Thread-local pointer to LogicState for gesture callbacks
    static LOGIC_STATE_PTR: std::cell::Cell<*const LogicState> = std::cell::Cell::new(std::ptr::null());
}

/// Convert HandlerType to V2 GestureType
/// Returns None for pointer events (not gestures)
#[cfg(not(target_arch = "wasm32"))]
fn to_v2_gesture_type(handler_type: HandlerType) -> Option<V2GestureType> {
    match handler_type {
        // All tap counts unified to Tap type - max_tap_count handles the difference
        HandlerType::Tap(_) => Some(V2GestureType::Tap),
        HandlerType::LongPress => Some(V2GestureType::LongPress),
        HandlerType::Pan => Some(V2GestureType::Pan),
        HandlerType::Scale => Some(V2GestureType::Scale),
        HandlerType::Rotation => Some(V2GestureType::Rotation),
        // Pointer events are not gestures, they are handled separately
        HandlerType::PointerDown | HandlerType::PointerUp => None,
    }
}

/// Build V2 GestureConfig from HandlerRegistry
#[cfg(not(target_arch = "wasm32"))]
fn build_v2_config(node_id: u32, registry: &HandlerRegistry) -> V2GestureConfig {
    let gestures = registry.get_node_gestures(node_id);
    let registered_types: Vec<V2GestureType> = gestures
        .into_iter()
        .filter_map(to_v2_gesture_type)
        .collect();

    // Determine max_tap_count from registry (supports single/double/triple/etc)
    let max_tap_count = registry.get_max_tap_count(node_id).max(1);

    V2GestureConfig {
        node_id,
        registered_types,
        max_tap_count,
        ..Default::default()
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

            // Set shared buffer pointer in SharedState for layout sync
            if let Some(bptr) = bptr {
                let mem = unsafe { &mut *logic._rt.memory_mut() };
                let shared_buffer_ptr = unsafe {
                    mem.as_mut_ptr().add(bptr as usize) as *const dyxel_shared::SharedBuffer
                };

                if let Ok(mut state) = logic.shared_state.lock() {
                    state.set_shared_buffer_ptr(
                        shared_buffer_ptr as *mut dyxel_shared::SharedBuffer,
                    );
                }
            }

            // Create gesture router
            let new_router = GestureRouter::new();

            *router.borrow_mut() = Some(new_router);
            log::info!("GestureRouter initialized");
        }
    });
}

/// Ensure node is registered in V2 router
#[cfg(not(target_arch = "wasm32"))]
fn ensure_node_registered_v2(router: &mut GestureRouter, node_id: u32) {
    let registry = get_handler_registry().lock().unwrap();
    let gestures = registry.get_node_gestures(node_id);

    log::info!(
        "ensure_node_registered_v2: node_id={}, gestures={:?}",
        node_id,
        gestures
    );

    if !gestures.is_empty() {
        let config = build_v2_config(node_id, &registry);
        log::info!(
            "ensure_node_registered_v2: registering node {} with types={:?} max_tap_count={}",
            node_id,
            config.registered_types,
            config.max_tap_count
        );
        router.register_node_gestures(node_id, config);
    } else {
        log::warn!(
            "ensure_node_registered_v2: node {} has NO gestures - will not receive tap events!",
            node_id
        );
    }
    drop(registry);
}

/// V2 Gesture Event Type constants
const GESTURE_TYPE_TAP: u8 = 0;
const GESTURE_TYPE_LONG_PRESS: u8 = 1;
const GESTURE_TYPE_PAN: u8 = 2;
const GESTURE_TYPE_SCALE: u8 = 3;
const GESTURE_TYPE_ROTATION: u8 = 4;

/// V2 Gesture Phase constants
const GESTURE_PHASE_BEGAN: u8 = 0;
const GESTURE_PHASE_CHANGED: u8 = 1;
const GESTURE_PHASE_ENDED: u8 = 2;
const GESTURE_PHASE_CANCELLED: u8 = 3;

#[allow(dead_code)]
/// Encode f32 to u32 for payload (preserves 16-bit precision for delta values)
fn encode_f32_to_u32(v: f32) -> u32 {
    // Scale by 1000 and round to preserve 3 decimal places
    (v * 1000.0).round() as i32 as u32
}

#[allow(dead_code)]
/// Decode u32 to f32
fn decode_u32_to_f32(v: u32) -> f32 {
    (v as i32) as f32 / 1000.0
}

#[cfg(not(target_arch = "wasm32"))]
fn dispatch_gesture_event_v2(logic: &LogicState, event: dyxel_gesture::GestureEvent) {
    use crate::handler_registry::HandlerType;
    use dyxel_gesture::GestureEventType;
    use dyxel_shared::push_command;

    let bptr = match *logic.shared_buffer_ptr.lock().unwrap() {
        Some(ptr) => ptr,
        None => {
            log::warn!("dispatch_gesture_event_v2: No shared buffer pointer");
            return;
        }
    };

    // Build bubble path from target to root using LogicState
    let bubble_path = build_bubble_path(event.target_node_id, logic);

    // Map event type to V2 encoding
    let (event_type, phase) = match event.event_type {
        GestureEventType::Tap => (GESTURE_TYPE_TAP, GESTURE_PHASE_ENDED),
        GestureEventType::LongPressStart => (GESTURE_TYPE_LONG_PRESS, GESTURE_PHASE_BEGAN),
        GestureEventType::LongPressEnd => (GESTURE_TYPE_LONG_PRESS, GESTURE_PHASE_ENDED),
        GestureEventType::PanStart => (GESTURE_TYPE_PAN, GESTURE_PHASE_BEGAN),
        GestureEventType::PanUpdate => (GESTURE_TYPE_PAN, GESTURE_PHASE_CHANGED),
        GestureEventType::PanEnd => (GESTURE_TYPE_PAN, GESTURE_PHASE_ENDED),
        GestureEventType::ScaleStart => (GESTURE_TYPE_SCALE, GESTURE_PHASE_BEGAN),
        GestureEventType::ScaleUpdate => (GESTURE_TYPE_SCALE, GESTURE_PHASE_CHANGED),
        GestureEventType::ScaleEnd => (GESTURE_TYPE_SCALE, GESTURE_PHASE_ENDED),
        GestureEventType::RotationStart => (GESTURE_TYPE_ROTATION, GESTURE_PHASE_BEGAN),
        GestureEventType::RotationUpdate => (GESTURE_TYPE_ROTATION, GESTURE_PHASE_CHANGED),
        GestureEventType::RotationEnd => (GESTURE_TYPE_ROTATION, GESTURE_PHASE_ENDED),
    };

    // Determine handler type for registry lookup
    let handler_type = match event.event_type {
        GestureEventType::Tap => Some(HandlerType::Tap(event.tap_count.max(1))),
        GestureEventType::LongPressStart | GestureEventType::LongPressEnd => {
            Some(HandlerType::LongPress)
        }
        GestureEventType::PanStart | GestureEventType::PanUpdate | GestureEventType::PanEnd => {
            Some(HandlerType::Pan)
        }
        GestureEventType::ScaleStart
        | GestureEventType::ScaleUpdate
        | GestureEventType::ScaleEnd => Some(HandlerType::Scale),
        GestureEventType::RotationStart
        | GestureEventType::RotationUpdate
        | GestureEventType::RotationEnd => Some(HandlerType::Rotation),
    };

    // SAFETY: This is called from the Logic Thread where the runtime is valid
    unsafe {
        let mem = &mut *logic._rt.memory_mut();
        let shared_buffer =
            &mut *(mem.as_mut_ptr().add(bptr as usize) as *mut dyxel_shared::SharedBuffer);

        // Find target node via HandlerRegistry
        let (target_node, is_text_input) = if let Some(ht) = handler_type {
            let registry = get_handler_registry().lock().unwrap();
            let handler_node = registry.find_handler(&bubble_path, ht);

            // Check if the clicked node is a TextInput (has TextInput registry entry)
            let is_input = handler_node.map_or(false, |node_id| {
                crate::text_input::TextInputManager::with(|m| m.get(node_id).is_some())
            });

            drop(registry);

            log::debug!("dispatch_gesture_event_v2: bubble_path={:?}, handler_type={:?}, handler_node={:?}, is_text_input={}",
                bubble_path, ht, handler_node, is_input);

            (handler_node.unwrap_or(event.target_node_id), is_input)
        } else {
            (event.target_node_id, false)
        };

        // === Blur unfocused TextInputs when tapping outside ===
        // If this is a tap gesture and the target is NOT a TextInput, blur any focused TextInput
        let mut blurred_id = 0u32;
        if matches!(event.event_type, GestureEventType::Tap) && !is_text_input {
            crate::text_input::TextInputManager::with(|m| {
                let focused_id = m.focused_id();
                if focused_id != 0 {
                    log::debug!(
                        "Tapping outside TextInput (node {}), blurring focused TextInput {}",
                        target_node,
                        focused_id
                    );
                    m.set_focused(focused_id, false);
                    blurred_id = focused_id;

                    // Sync to WASM: clear focus and hide keyboard
                    push_command!(
                        shared_buffer,
                        SetTextInputFocused,
                        focused_id,
                        0u8
                    );
                    push_command!(shared_buffer, HideTextInputKeyboard);
                }
            });
            // Sync to renderer immediately after blur to ensure cursor and border disappear
            if blurred_id != 0 {
                sync_to_renderer();
            }
        }

        // Encode payload based on gesture type
        let payload = match event.event_type {
            GestureEventType::Tap => event.tap_count as u32,
            GestureEventType::PanUpdate => {
                // Pack delta_x and delta_y into single u32 (16 bits each)
                let dx = ((event.delta_x * 100.0) as i16) as u16;
                let dy = ((event.delta_y * 100.0) as i16) as u16;
                ((dx as u32) << 16) | (dy as u32)
            }
            GestureEventType::ScaleUpdate => {
                // Pack scale (8-bit integer part) + scale_delta (8-bit signed)
                let scale_int = event.scale.clamp(0.0, 25.5) as u8;
                let scale_frac = ((event.scale.fract() * 256.0) as u8) as u32;
                ((scale_int as u32) << 24) | (scale_frac << 16)
            }
            GestureEventType::RotationUpdate => {
                // Pack rotation in radians as i16 (scaled by 1000 for precision)
                // Range: -32.768 to +32.768 radians (~-1875 to +1875 degrees)
                let rotation_scaled = (event.delta_x * 1000.0) as i16;
                ((rotation_scaled as u16) as u32) << 16
            }
            _ => 0u32,
        };

        // Send unified V2 gesture event
        // Use V2Ex if payload is non-zero, otherwise use V2
        log::debug!(
            "DyxelInput: Sending GestureEventV2 to node {} type={} phase={}",
            target_node,
            event_type,
            phase
        );
        if payload != 0 {
            push_command!(
                shared_buffer,
                GestureEventV2Ex,
                target_node,
                event_type,
                phase,
                event.x,
                event.y,
                payload
            );
        } else {
            push_command!(
                shared_buffer,
                GestureEventV2,
                target_node,
                event_type,
                phase,
                event.x,
                event.y
            );
        }

        // Log for debugging
        let gesture_name = match event_type {
            GESTURE_TYPE_TAP => "Tap",
            GESTURE_TYPE_LONG_PRESS => "LongPress",
            GESTURE_TYPE_PAN => "Pan",
            GESTURE_TYPE_SCALE => "Scale",
            GESTURE_TYPE_ROTATION => "Rotation",
            _ => "Unknown",
        };
        let phase_name = match phase {
            GESTURE_PHASE_BEGAN => "Began",
            GESTURE_PHASE_CHANGED => "Changed",
            GESTURE_PHASE_ENDED => "Ended",
            GESTURE_PHASE_CANCELLED => "Cancelled",
            _ => "Unknown",
        };

        if matches!(phase, GESTURE_PHASE_BEGAN | GESTURE_PHASE_ENDED)
            || event_type == GESTURE_TYPE_TAP
        {
            log::info!(
                "GestureV2: {} {} on node {} at ({:.1},{:.1})",
                gesture_name,
                phase_name,
                target_node,
                event.x,
                event.y
            );
        }
    }
}

/// Build bubble path from target node to root
#[cfg(not(target_arch = "wasm32"))]
fn build_bubble_path(target_node: u32, logic: &LogicState) -> Vec<u32> {
    let mut path = vec![target_node];

    // Walk up parent chain using SharedState
    // This queries the Host-side tree structure
    if let Ok(state) = logic.shared_state.lock() {
        let mut current = target_node;

        // Traverse parent chain until we reach root (parent_id == 0)
        while current != 0 {
            if let Some(node) = state.nodes.get(&current) {
                if node.parent_id != 0 && node.parent_id != current {
                    path.push(node.parent_id);
                    current = node.parent_id;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    path
}

#[cfg(not(target_arch = "wasm32"))]
fn process_input_internal(logic: &mut LogicState, event: InputEvent) {
    // InputEventType and RawInputEvent not used in V2
    use dyxel_gesture::{PointerEvent, PointerEventType};

    log::debug!("DyxelInput: process_input_internal event={:?}", event);

    // Ensure GestureRouter is initialized
    ensure_gesture_router_initialized(logic);

    // Convert InputEvent to V2 PointerEvent
    let (event_type, pointer_id, x, y, pressure) = match event {
        InputEvent::TouchDown { x, y } => (PointerEventType::Down, 0, x, y, 1.0),
        InputEvent::TouchMove { x, y } => (PointerEventType::Move, 0, x, y, 1.0),
        InputEvent::TouchUp { x, y } => (PointerEventType::Up, 0, x, y, 0.0),
        InputEvent::PointerDown {
            pointer_id,
            x,
            y,
            pressure,
        } => (PointerEventType::Down, pointer_id, x, y, pressure),
        InputEvent::PointerMove { pointer_id, x, y } => {
            (PointerEventType::Move, pointer_id, x, y, 1.0)
        }
        InputEvent::PointerUp { pointer_id, x, y } => (PointerEventType::Up, pointer_id, x, y, 0.0),
        InputEvent::PointerCancel => (PointerEventType::Cancel, 0, 0.0, 0.0, 0.0),
    };

    // Get timestamp from host (microseconds)
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    // Hit test to find target node
    let target_node_id = GESTURE_ROUTER.with(|_router| {
        // First, ensure spatial hit tester is synced
        LOGIC_STATE_PTR.with(|ptr| {
            let logic_ptr = ptr.get();
            log::info!(
                "DyxelInput: LOGIC_STATE_PTR is_null={}",
                logic_ptr.is_null()
            );
            if !logic_ptr.is_null() {
                unsafe {
                    let logic = &*logic_ptr;
                    let bptr = *logic.shared_buffer_ptr.lock().unwrap();
                    log::info!("DyxelInput: shared_buffer_ptr={:?}", bptr);
                    if let Some(bptr) = bptr {
                        let mem = &mut *logic._rt.memory_mut();
                        let shared_buffer_ptr = mem.as_mut_ptr().add(bptr as usize)
                            as *const dyxel_shared::SharedBuffer;
                        let mut tester = SpatialHitTester::new(shared_buffer_ptr);
                        tester.sync();
                        let result = tester.hit_test(x, y);
                        log::info!(
                            "DyxelInput: Hit test at ({:.1},{:.1}) -> node {} inside={}",
                            x,
                            y,
                            result.node_id,
                            result.is_inside
                        );
                        return result.node_id;
                    } else {
                        log::warn!("DyxelInput: shared_buffer_ptr is None!");
                    }
                }
            } else {
                log::warn!("DyxelInput: LOGIC_STATE_PTR is null!");
            }
            log::warn!("DyxelInput: Hit test failed, returning 0");
            0
        })
    });

    // Create V2 PointerEvent
    let pointer_event = PointerEvent {
        event_type,
        pointer_id,
        timestamp_us: timestamp,
        x,
        y,
        pressure,
        target_node_id,
    };

    // Route through V2 GestureRouter
    GESTURE_ROUTER.with(|router_cell| {
        if let Some(ref mut router) = *router_cell.borrow_mut() {
            let bubble_path = build_bubble_path(target_node_id, logic);
            log::debug!("DyxelInput: bubble_path={:?}", bubble_path);

            // Ensure all nodes in bubble path are registered
            for &node_id in &bubble_path {
                if node_id != 0 {
                    ensure_node_registered_v2(router, node_id);
                }
            }

            log::trace!(
                "DyxelInput: Routing ptr={} type={:?} target={}",
                pointer_event.pointer_id,
                pointer_event.event_type,
                target_node_id
            );

            // Process timer-based events FIRST (before pointer event)
            // This ensures recognizers like LongPress can trigger before PointerUp causes them to fail
            let now = Instant::now();
            let timer_events = router.tick(now);

            // Process event and get gesture events
            let gesture_events =
                router.route_pointer_event_with_path(&pointer_event, bubble_path.clone());

            // Dispatch all events (timer events first, then pointer events)
            log::debug!(
                "DyxelInput: Dispatching {} timer events and {} gesture events",
                timer_events.len(),
                gesture_events.len()
            );
            for event in timer_events.into_iter().chain(gesture_events.into_iter()) {
                log::debug!(
                    "DyxelInput: Dispatching gesture event {:?}",
                    event.event_type
                );
                dispatch_gesture_event_v2(logic, event);
            }

            // Also dispatch raw pointer events for press effects
            // Check if any node in bubble path has PointerDown/PointerUp handlers
            dispatch_pointer_event_v2(logic, &pointer_event, &bubble_path);
        } else {
            log::warn!("DyxelInput: GestureRouter not initialized!");
        }
    });
}

// =============== Pointer Event Dispatch (for press effects) ===============

/// Dispatch raw pointer events for press effects
fn dispatch_pointer_event_v2(
    logic: &LogicState,
    event: &dyxel_gesture::PointerEvent,
    bubble_path: &[u32],
) {
    use crate::handler_registry::HandlerType;
    use dyxel_gesture::PointerEventType;
    use dyxel_shared::push_command;

    let bptr = match *logic.shared_buffer_ptr.lock().unwrap() {
        Some(ptr) => ptr,
        None => return,
    };

    // Determine handler type based on pointer event type
    let handler_type = match event.event_type {
        PointerEventType::Down => Some(HandlerType::PointerDown),
        PointerEventType::Up => Some(HandlerType::PointerUp),
        PointerEventType::Cancel => Some(HandlerType::PointerUp), // Treat cancel as up
        _ => None,
    };

    let ht = match handler_type {
        Some(ht) => ht,
        None => return, // Only handle Down/Up/Cancel
    };

    // Find target node via HandlerRegistry
    let registry = get_handler_registry().lock().unwrap();
    let target_node = registry.find_handler(bubble_path, ht);
    drop(registry);

    // Get the actual hit test node (first in bubble path, or 0 if empty)
    let hit_node = bubble_path.first().copied().unwrap_or(0);

    // Check if hit_node is the currently focused TextInput
    let is_hitting_focused_input = crate::text_input::TextInputManager::with(|m| {
        let focused_id = m.focused_id();
        focused_id != 0 && focused_id == hit_node
    });

    // If no handler registered (tapping blank area), trigger blur for focused TextInput
    let target_node = match target_node {
        Some(node) => node,
        None => {
            // On PointerDown/PointerUp with no handler (touching blank area), blur any focused TextInput
            // But only if we're not hitting the currently focused input itself
            if !is_hitting_focused_input &&
               matches!(event.event_type, PointerEventType::Down | PointerEventType::Up | PointerEventType::Cancel) {
                crate::text_input::TextInputManager::with(|m| {
                    let focused_id = m.focused_id();
                    if focused_id != 0 {
                        log::debug!(
                            "Touch outside focused input (hit_node={}), blurring TextInput {}",
                            hit_node,
                            focused_id
                        );
                        m.set_focused(focused_id, false);

                        // Sync to WASM: clear focus and hide keyboard
                        unsafe {
                            let mem = &mut *logic._rt.memory_mut();
                            let shared_buffer =
                                &mut *(mem.as_mut_ptr().add(bptr as usize) as *mut dyxel_shared::SharedBuffer);

                            push_command!(
                                shared_buffer,
                                SetTextInputFocused,
                                focused_id,
                                0u8
                            );
                            push_command!(shared_buffer, HideTextInputKeyboard);
                        }
                    }
                });
                // Sync to renderer immediately after blur
                crate::text_input::sync_to_renderer();
            }
            return;
        }
    };

    // Send pointer event via GestureEventV2 with custom event type
    // Using event types 100+ for pointer events (reserve 100-127)
    const POINTER_EVENT_DOWN: u8 = 100;
    const POINTER_EVENT_UP: u8 = 101;

    let event_type = match event.event_type {
        PointerEventType::Down => POINTER_EVENT_DOWN,
        PointerEventType::Up => POINTER_EVENT_UP,
        PointerEventType::Cancel => POINTER_EVENT_UP,
        _ => return,
    };

    unsafe {
        let mem = &mut *logic._rt.memory_mut();
        let shared_buffer =
            &mut *(mem.as_mut_ptr().add(bptr as usize) as *mut dyxel_shared::SharedBuffer);

        push_command!(
            shared_buffer,
            GestureEventV2,
            target_node,
            event_type,
            0u8,
            event.x,
            event.y
        );
    }
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
                        log::debug!("LogicThread: loop start, lifecycle={:?}", lifecycle);
                        // Clear any pending VSync signals to prevent frame lag accumulation
                        // Logic Thread should sync with latest VSync, not old ones
                        log::debug!("LogicThread: clearing VSync signals...");
                        while render_complete_rx.try_recv().is_ok() {}
                        
                        // Receive message (block when stopped/paused to save CPU)
                        log::debug!("LogicThread: checking for messages...");
                        let msg_res = if lifecycle == Lifecycle::Running {
                            // Running: non-blocking check then wait for VSync if no message
                            match logic_rx.try_recv() {
                                Ok(msg) => {
                                    log::debug!("LogicThread: received message");
                                    Ok(msg)
                                }
                                Err(std::sync::mpsc::TryRecvError::Empty) => {
                                    log::debug!("LogicThread: no message, executing tick...");
                                    // No message, execute tick and sleep
                                    if let Some(ref mut l) = logic_opt {
                                        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
                                        {
                                            use crate::runtime::{process_commands, sync_layout_to_wasm, is_render_needed, clear_dirty_tracker};
                                            
                                            // Execute WASM tick (produces commands)
                                            log::debug!("LogicThread: acquiring tick_fn lock...");
                                            
                                            // Process gesture router timers
                                            GESTURE_ROUTER.with(|router_cell| {
                                                if let Some(router) = router_cell.borrow_mut().as_mut() {
                                                    let timer_events = router.tick(Instant::now());
                                                    for event in timer_events {
                                                        dispatch_gesture_event_v2(l, event);
                                                    }
                                                }
                                            });

                                            let tick_opt = l.tick_fn.lock().unwrap();
                                            log::debug!("LogicThread: tick_fn lock acquired");
                                            if let Some(tick) = tick_opt.as_ref() {
                                                log::debug!("LogicThread: calling tick...");
                                                if let Err(e) = tick.call() {
                                                    log::error!("LogicThread: WASM tick failed: {}", e);
                                                }
                                                log::debug!("LogicThread: tick returned");
                                            } else {
                                                log::warn!("LogicThread: tick_fn is None, skipping tick");
                                            }
                                            drop(tick_opt);
                                            
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
                                    // After tick/VSync, check for input messages before continuing
                                    // This prevents input events from being delayed by VSync wait
                                    log::debug!("LogicThread: tick/VSync complete, continuing loop");
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
                        log::trace!("LogicThread: msg_res={:?}, lifecycle={:?}", msg_res.is_ok(), lifecycle);
                        if let Ok(msg) = msg_res {
                            match &msg {
                                LogicMessage::SetReady(_) => log::debug!("LogicThread: msg type=SetReady"),
                                LogicMessage::Input(_) => log::trace!("LogicThread: msg type=Input"),
                                LogicMessage::LoadWasm(_) => log::debug!("LogicThread: msg type=LoadWasm"),
                                LogicMessage::Pause => log::debug!("LogicThread: msg type=Pause"),
                                LogicMessage::Resume => log::debug!("LogicThread: msg type=Resume"),
                                LogicMessage::Shutdown => log::info!("LogicThread: msg type=Shutdown"),
                                LogicMessage::TextInput { node_id, text } => log::debug!("LogicThread: msg type=TextInput node_id={} text_len={}", node_id, text.len()),
                            }
                            match msg {
                                LogicMessage::SetReady(l) => {
                                    log::info!("LogicThread: Received SetReady, initializing...");
                                    logic_opt = Some(l);
                                    lifecycle = Lifecycle::Running;
                                    log::info!("LogicThread: Running now!");
                                }
                                LogicMessage::Input(event) => {
                                    log::info!("LogicThread: Received Input event={:?}, logic_opt={}", event, logic_opt.is_some());
                                    if let Some(ref mut l) = logic_opt { process_input_internal(l, event); }
                                }
                                LogicMessage::TextInput { node_id, text } => {
                                    log::info!("LogicThread: Received TextInput node_id={} text_len={}", node_id, text.len());
                                    if let Some(ref mut l) = logic_opt {
                                        // Get the shared buffer pointer and push text
                                        if let Ok(state) = l.shared_state.lock() {
                                            if let Some(buffer_ptr) = state.get_shared_buffer_ptr() {
                                                if !buffer_ptr.is_null() {
                                                    // SAFETY: We assume the pointer is valid when non-null
                                                    let buffer = unsafe { &mut *buffer_ptr };
                                                    // Push text to input buffer for the focused node
                                                    buffer.push_input_text(node_id, &text);
                                                    log::info!("LogicThread: Pushed text to input buffer for node_id={}", node_id);
                                                } else {
                                                    log::warn!("LogicThread: TextInput - shared buffer ptr is null");
                                                }
                                            } else {
                                                log::warn!("LogicThread: TextInput - cannot get shared buffer ptr");
                                            }
                                        } else {
                                            log::warn!("LogicThread: TextInput - failed to lock shared state");
                                        }
                                    } else {
                                        log::warn!("LogicThread: TextInput - logic_opt is None");
                                    }
                                }
                                LogicMessage::LoadWasm(path) => {
                                    log::info!("LogicThread: Processing LoadWasm...");
                                    if let Some(ref mut l) = logic_opt { 
                                        log::info!("LogicThread: Calling load_wasm...");
                                        if let Err(e) = l.load_wasm(path) {
                                            log::error!("LogicThread: LoadWasm failed: {}", e);
                                        } else {
                                            log::info!("LogicThread: LoadWasm completed successfully");
                                        }
                                        log::info!("LogicThread: LoadWasm block done");
                                    } else {
                                        log::warn!("LogicThread: LoadWasm - logic_opt is None");
                                    }
                                    log::info!("LogicThread: LoadWasm message processing complete");
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
                    if let Some(v_ctx) = render.context.downcast_ref::<vello::util::RenderContext>()
                    {
                        *self.instance.lock().unwrap() = Some(Box::new(v_ctx.instance.clone()));
                    }

                    // Set the logic thread sender for the bridge (for keyboard input handling)
                    // Note: init_bridge is called in setup_engine, so bridge is ready now
                    if let Some(ref tx) = *self.logic_tx.lock().unwrap() {
                        crate::set_bridge_logic_tx(tx.clone());
                        log::info!("[DyxelHost] Bridge logic_tx set for keyboard input");
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
        {
            let host_ptr = format!("{:p}", self);
            let logic_tx_guard = self.logic_tx.lock().unwrap();
            if let Some(tx) = &*logic_tx_guard {
                log::info!(
                    "DyxelInput: on_pointer_down pid={} x={:.1} y={:.1}",
                    pointer_id,
                    x,
                    y
                );
                match tx.send(LogicMessage::Input(InputEvent::PointerDown {
                    pointer_id,
                    x,
                    y,
                    pressure,
                })) {
                    Ok(_) => {}
                    Err(e) => log::error!("DyxelInput: Failed to send message: {}", e),
                }
            } else {
                log::warn!("DyxelInput: logic_tx is None, host={}", host_ptr);
            }
        }
    }

    /// 指针移动（支持多指）
    pub fn on_pointer_move(&self, pointer_id: u32, x: f32, y: f32) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            log::trace!(
                "DyxelInput: on_pointer_move pid={} x={:.1} y={:.1}",
                pointer_id,
                x,
                y
            );
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
            log::debug!(
                "DyxelInput: on_pointer_up pid={} x={:.1} y={:.1}",
                pointer_id,
                x,
                y
            );
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

    /// Get the shared state for bridge initialization
    /// This is needed for main thread input handling (e.g., keyboard events)
    pub fn shared_state_for_bridge(&self) -> SharedPtr<SharedMutex<SharedState>> {
        self.shared_state.clone()
    }

    /// Send text input to the logic thread for processing
    /// Called from main thread when keyboard input is received
    pub fn send_text_input(&self, node_id: u32, text: String) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            match tx.send(LogicMessage::TextInput { node_id, text }) {
                Ok(_) => log::debug!("send_text_input: sent to logic thread"),
                Err(e) => log::error!("send_text_input: Failed to send: {:?}", e),
            }
        } else {
            log::warn!("send_text_input: logic_tx is None");
        }
    }

    /// Setup bridge for main thread keyboard input handling
    /// Must be called on the main thread after prepare_engine is complete
    pub fn setup_main_thread_bridge(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.logic_tx.lock().unwrap() {
            crate::set_bridge_logic_tx(tx.clone());
            log::info!("[DyxelHost] Main thread bridge setup complete");
        } else {
            log::warn!("[DyxelHost] setup_main_thread_bridge: logic_tx is None");
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
                        let mut target_opt: Option<dyxel_render_api::SurfaceTargetHandle> =
                            Some(target);
                        if let Some(wgpu_target) = target_opt
                            .take()
                            .unwrap()
                            .into_inner::<vello::wgpu::SurfaceTarget<'static>>(
                        ) {
                            match instance.create_surface(wgpu_target) {
                                Ok(s) => (None, Some(dyxel_render_api::SurfaceHandle::new(s))),
                                Err(e) => {
                                    log::error!(
                                        "setup: Failed to create surface on main thread: {}",
                                        e
                                    );
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
