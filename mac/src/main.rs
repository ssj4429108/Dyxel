// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;
use winit::{event::*, event_loop::{ControlFlow, EventLoop}, window::WindowBuilder};
use dyxel_core::DyxelHost;
use kurbo::Vec2;
use std::thread;

mod touch;
use touch::TouchTracker;

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let event_loop = EventLoop::new()?;
    let window = Arc::new(WindowBuilder::new()
        .with_title("Dyxel Native (macOS)")
        .with_inner_size(winit::dpi::LogicalSize::new(1024.0, 768.0))
        .build(&event_loop)?);

    let host = DyxelHost::new();

    // Start preparing engine in background thread
    let h_init = host.clone();
    thread::spawn(move || {
        pollster::block_on(h_init.prepare_engine(".".to_string()));
        pollster::block_on(h_init.load_wasm("guest.wasm".to_string()));
    });

    let mut mouse_pos = Vec2::ZERO;
    let mut mouse_pressed = false;
    let mut surface_setup_done = false;
    let mut touch_tracker = TouchTracker::new();

    // Gesture state for simulating two-finger touch
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum GestureMode {
        None,
        Scale,      // Two fingers horizontal, distance changes with mouse x
        Rotation,   // Two fingers rotate around center with mouse movement
    }
    let mut gesture_mode = GestureMode::None;
    let mut gesture_center = Vec2::ZERO;
    let mut gesture_finger1_pos = Vec2::ZERO;
    let mut gesture_finger2_pos = Vec2::ZERO;
    let mut last_mouse_pos = Vec2::ZERO;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent { event: WindowEvent::Resized(new_size), .. } => {
                if surface_setup_done {
                    host.resize_native(new_size.width, new_size.height);
                }
            }
            Event::AboutToWait => {
                if !surface_setup_done && host.is_ready() {
                    let h = host.clone();
                    let w = window.clone();
                    let size = w.inner_size();
                    let wgpu_target: vello::wgpu::SurfaceTarget<'static> = w.clone().into();
                    let target_handle = dyxel_render_api::SurfaceTargetHandle::new(wgpu_target);
                    pollster::block_on(h.setup(
                        target_handle,
                        size.width,
                        size.height,
                        None
                    ));
                    surface_setup_done = true;
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                host.stop_native();
                elwt.exit();
            }

            // Multi-touch support via winit's Touch event
            // NOTE: On macOS, this only works if the window system is configured
            // to send touch events (usually requires Info.plist configuration
            // or specific NSTouchBar/NSTouch handling)
            Event::WindowEvent { event: WindowEvent::Touch(touch), .. } => {
                let native_id = touch.id as u64;
                let x = touch.location.x;
                let y = touch.location.y;

                match touch.phase {
                    winit::event::TouchPhase::Started => {
                        let pid = touch_tracker.get_pointer_id(native_id);
                        log::debug!("Touch began: native={} pointer={} pos=({:.1},{:.1})", native_id, pid, x, y);
                        host.on_pointer_down(pid, x as f32, y as f32, 1.0);
                    }
                    winit::event::TouchPhase::Moved => {
                        let pid = touch_tracker.get_pointer_id(native_id);
                        host.on_pointer_move(pid, x as f32, y as f32);
                    }
                    winit::event::TouchPhase::Ended => {
                        if let Some(pid) = touch_tracker.release_touch(native_id) {
                            log::debug!("Touch ended: native={} pointer={} pos=({:.1},{:.1})", native_id, pid, x, y);
                            host.on_pointer_up(pid, x as f32, y as f32);
                        }
                    }
                    winit::event::TouchPhase::Cancelled => {
                        if let Some(pid) = touch_tracker.release_touch(native_id) {
                            log::debug!("Touch cancelled: native={} pointer={}", native_id, pid);
                            host.on_pointer_up(pid, x as f32, y as f32);
                        }
                    }
                }
            }

            // Mouse fallback (only active when no touches)
            Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } => {
                mouse_pos = Vec2::new(position.x, position.y);

                // Handle gesture simulation mode
                match gesture_mode {
                    GestureMode::Scale => {
                        // Scale: mouse x movement changes finger distance
                        // Move mouse right = zoom in (fingers spread apart)
                        // Move mouse left = zoom out (fingers come together)
                        let _delta_x = mouse_pos.x - last_mouse_pos.x;
                        let center = gesture_center;

                        // Calculate new half_distance based on mouse movement from start
                        // Start with 50px, add accumulated delta
                        let base_half_dist = 50.0;
                        let movement = (mouse_pos.x - gesture_center.x).clamp(-100.0, 100.0);
                        let half_distance = (base_half_dist + movement).max(10.0);

                        gesture_finger1_pos = Vec2::new(center.x - half_distance, center.y);
                        gesture_finger2_pos = Vec2::new(center.x + half_distance, center.y);

                        host.on_pointer_move(1000, gesture_finger1_pos.x as f32, gesture_finger1_pos.y as f32);
                        host.on_pointer_move(1001, gesture_finger2_pos.x as f32, gesture_finger2_pos.y as f32);
                        log::debug!("Scale gesture: fingers at {:.1} and {:.1}, distance={:.1}", gesture_finger1_pos.x, gesture_finger2_pos.x, half_distance * 2.0);
                    }
                    GestureMode::Rotation => {
                        // Rotation: mouse movement rotates fingers around center
                        // Use mouse position relative to center to determine rotation angle
                        let dx = mouse_pos.x - gesture_center.x;
                        let dy = mouse_pos.y - gesture_center.y;

                        // Calculate angle from center to mouse position
                        // Use this angle to place the two fingers opposite each other
                        let angle = dy.atan2(dx);
                        let distance = 50.0; // Fixed distance from center

                        gesture_finger1_pos = Vec2::new(
                            gesture_center.x - distance * angle.cos(),
                            gesture_center.y - distance * angle.sin()
                        );
                        gesture_finger2_pos = Vec2::new(
                            gesture_center.x + distance * angle.cos(),
                            gesture_center.y + distance * angle.sin()
                        );

                        host.on_pointer_move(1000, gesture_finger1_pos.x as f32, gesture_finger1_pos.y as f32);
                        host.on_pointer_move(1001, gesture_finger2_pos.x as f32, gesture_finger2_pos.y as f32);
                        log::debug!("Rotation gesture: angle={:.2} rad ({:.1} deg)", angle, angle.to_degrees());
                    }
                    GestureMode::None => {
                        // Normal mouse move
                        if mouse_pressed && !touch_tracker.has_active_touches() {
                            host.on_pointer_move(0, mouse_pos.x as f32, mouse_pos.y as f32);
                        }
                    }
                }

                last_mouse_pos = mouse_pos;
            }
            Event::WindowEvent { event: WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. }, .. } => {
                mouse_pressed = true;
                if !touch_tracker.has_active_touches() {
                    host.on_pointer_down(0, mouse_pos.x as f32, mouse_pos.y as f32, 1.0);
                }
            }
            Event::WindowEvent { event: WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. }, .. } => {
                mouse_pressed = false;
                if !touch_tracker.has_active_touches() {
                    host.on_pointer_up(0, mouse_pos.x as f32, mouse_pos.y as f32);
                }
            }

            Event::WindowEvent { event: WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                logical_key: winit::keyboard::Key::Character(c),
                ..
            }, .. }, .. } if c == "p" || c == "P" => {
                host.toggle_perf_overlay();
            }
            // Mouse wheel with Option key to simulate pinch gesture
            // This is a workaround for winit 0.29 which doesn't support PinchGesture on macOS
            Event::WindowEvent { event: WindowEvent::MouseWheel { delta, .. }, .. } => {
                // Extract scroll delta values from MouseScrollDelta enum
                let (delta_x, delta_y): (f32, f32) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (x, y),
                    winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x as f32, pos.y as f32),
                };

                // Check if Option key is held (modifiers state would need to be tracked)
                // For now, use large scroll deltas as a heuristic for pinch
                let is_pinch = delta_y.abs() > 10.0 || delta_x.abs() > 10.0;

                if is_pinch {
                    let pos = mouse_pos;
                    let base_distance = 50.0;
                    // Convert scroll delta to scale factor
                    let scale_factor: f32 = 1.0 + (delta_y / 100.0);
                    let scale = scale_factor.clamp(0.5, 3.0);

                    if gesture_mode == GestureMode::None {
                        // Begin pinch simulation
                        let distance = base_distance * scale as f64;
                        gesture_finger1_pos = Vec2::new(pos.x - distance / 2.0, pos.y);
                        gesture_finger2_pos = Vec2::new(pos.x + distance / 2.0, pos.y);
                        gesture_center = pos;
                        gesture_mode = GestureMode::Scale;

                        host.on_pointer_down(1000, gesture_finger1_pos.x as f32, gesture_finger1_pos.y as f32, 1.0);
                        host.on_pointer_down(1001, gesture_finger2_pos.x as f32, gesture_finger2_pos.y as f32, 1.0);
                        log::debug!("Simulated pinch began: scale={:.3}", scale);
                    } else {
                        // Update pinch
                        let distance = base_distance * scale as f64;
                        let new_finger1 = Vec2::new(pos.x - distance / 2.0, pos.y);
                        let new_finger2 = Vec2::new(pos.x + distance / 2.0, pos.y);

                        host.on_pointer_move(1000, new_finger1.x as f32, new_finger1.y as f32);
                        host.on_pointer_move(1001, new_finger2.x as f32, new_finger2.y as f32);

                        gesture_finger1_pos = new_finger1;
                        gesture_finger2_pos = new_finger2;
                        log::debug!("Simulated pinch: scale={:.3}", scale);
                    }

                    // Schedule pinch end after a short delay (since we don't have a gesture end event)
                    // This is a simplification - in production, use a timer
                    if scale_factor < 0.1 || scale_factor > 2.5 {
                        if gesture_mode != GestureMode::None {
                            host.on_pointer_up(1000, gesture_finger1_pos.x as f32, gesture_finger1_pos.y as f32);
                            host.on_pointer_up(1001, gesture_finger2_pos.x as f32, gesture_finger2_pos.y as f32);
                            gesture_mode = GestureMode::None;
                            log::debug!("Simulated pinch ended (threshold)");
                        }
                    }
                }
            }

            // Key 's' + drag to simulate scale gesture
            // Key 'r' + drag to simulate rotation gesture
            Event::WindowEvent { event: WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                logical_key: winit::keyboard::Key::Character(c),
                ..
            }, .. }, .. } if c == "s" || c == "S" => {
                if gesture_mode == GestureMode::None {
                    let pos = mouse_pos;
                    gesture_finger1_pos = Vec2::new(pos.x - 50.0, pos.y);
                    gesture_finger2_pos = Vec2::new(pos.x + 50.0, pos.y);
                    gesture_center = pos;
                    gesture_mode = GestureMode::Scale;
                    last_mouse_pos = pos;
                    host.on_pointer_down(1000, gesture_finger1_pos.x as f32, gesture_finger1_pos.y as f32, 1.0);
                    host.on_pointer_down(1001, gesture_finger2_pos.x as f32, gesture_finger2_pos.y as f32, 1.0);
                    log::info!("Scale simulation mode ACTIVE: Move mouse horizontally to change finger distance, 'x' to exit");
                }
            }
            Event::WindowEvent { event: WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                logical_key: winit::keyboard::Key::Character(c),
                ..
            }, .. }, .. } if c == "r" || c == "R" => {
                if gesture_mode == GestureMode::None {
                    let pos = mouse_pos;
                    gesture_finger1_pos = Vec2::new(pos.x, pos.y - 50.0);
                    gesture_finger2_pos = Vec2::new(pos.x, pos.y + 50.0);
                    gesture_center = pos;
                    gesture_mode = GestureMode::Rotation;
                    last_mouse_pos = pos;
                    host.on_pointer_down(1000, gesture_finger1_pos.x as f32, gesture_finger1_pos.y as f32, 1.0);
                    host.on_pointer_down(1001, gesture_finger2_pos.x as f32, gesture_finger2_pos.y as f32, 1.0);
                    log::info!("Rotation simulation mode ACTIVE: Move mouse to rotate fingers, 'x' to exit");
                }
            }
            Event::WindowEvent { event: WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                logical_key: winit::keyboard::Key::Character(c),
                ..
            }, .. }, .. } if c == "x" || c == "X" => {
                if gesture_mode != GestureMode::None {
                    host.on_pointer_up(1000, gesture_finger1_pos.x as f32, gesture_finger1_pos.y as f32);
                    host.on_pointer_up(1001, gesture_finger2_pos.x as f32, gesture_finger2_pos.y as f32);
                    gesture_mode = GestureMode::None;
                    log::info!("Gesture simulation ended");
                }
            }

            Event::WindowEvent { event: WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                logical_key: winit::keyboard::Key::Character(c),
                ..
            }, .. }, .. } if c == "c" || c == "C" => {
                use std::sync::atomic::{AtomicBool, Ordering};
                static CONTINUOUS: AtomicBool = AtomicBool::new(false);
                let new_state = !CONTINUOUS.load(Ordering::Relaxed);
                CONTINUOUS.store(new_state, Ordering::Relaxed);
                host.set_continuous_render(new_state);
                println!("Continuous render mode: {}", if new_state { "ON" } else { "OFF" });
            }
            _ => {}
        }
    })?;

    Ok(())
}
