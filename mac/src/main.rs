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
                if mouse_pressed && !touch_tracker.has_active_touches() {
                    host.on_pointer_move(0, mouse_pos.x as f32, mouse_pos.y as f32);
                }
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
