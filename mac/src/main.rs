// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;
use winit::{event::*, event_loop::{ControlFlow, EventLoop}, window::WindowBuilder};
use dyxel_core::DyxelHost;
use kurbo::Vec2;
use std::thread;

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let event_loop = EventLoop::new()?;
    let window = Arc::new(WindowBuilder::new()
        .with_title("Dyxel Native (macOS)")
        .with_inner_size(winit::dpi::LogicalSize::new(1024.0, 768.0))
        .build(&event_loop)?);

    let host = DyxelHost::new();

    // 1. Start preparing engine in background thread without blocking main thread
    let h_init = host.clone();
    thread::spawn(move || {
        pollster::block_on(h_init.prepare_engine(".".to_string()));
        // Load business logic immediately after environment is ready, otherwise screen will be black
        pollster::block_on(h_init.load_wasm("guest.wasm".to_string()));
    });


    let mut mouse_pos = Vec2::ZERO;
    let mut surface_setup_done = false;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        match event {
            // Execute Setup when window is ready and engine is also ready
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } | Event::AboutToWait => {
                if !surface_setup_done && host.is_engine_ready() {
                    let h = host.clone();
                    let w = window.clone();
                    pollster::block_on(h.setup(
                        vello::wgpu::SurfaceTarget::from(w.clone()),
                        w.inner_size().width,
                        w.inner_size().height,
                        None
                    ));
                    surface_setup_done = true;
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                host.stop_native();
                elwt.exit();
            }
            Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } => {
                mouse_pos = Vec2::new(position.x, position.y);
            }
            Event::WindowEvent { event: WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. }, .. } => {
                host.on_touch(mouse_pos.x as f32, mouse_pos.y as f32);
            }
            _ => {}
        }
    })?;

    Ok(())
}
