use std::sync::Arc;
use winit::{event::*, event_loop::{ControlFlow, EventLoop}, window::WindowBuilder};
use host_core::VelloHost;
use kurbo::Vec2;
use std::thread;

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let event_loop = EventLoop::new()?;
    let window = Arc::new(WindowBuilder::new()
        .with_title("VelloView Native (macOS)")
        .with_inner_size(winit::dpi::LogicalSize::new(1024.0, 768.0))
        .build(&event_loop)?);

    let host = VelloHost::new();
    
    // 1. 在后台线程开始准备引擎，不阻塞主线程
    let h_init = host.clone();
    thread::spawn(move || {
        pollster::block_on(h_init.prepare_engine_async(".".to_string()));
    });

    let mut mouse_pos = Vec2::ZERO;
    let mut surface_setup_done = false;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        match event {
            // 当窗口准备好且引擎也准备好时，执行 Setup
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } | Event::AboutToWait => {
                if !surface_setup_done && host.is_engine_ready() {
                    let h = host.clone();
                    let w = window.clone();
                    // setup 必须在主线程调用 (Metal 限制)
                    // 由于引擎已 Ready，此处的 block_on 会非常快
                    pollster::block_on(h.setup(
                        vello::wgpu::SurfaceTarget::from(w.clone()),
                        w.inner_size().width,
                        w.inner_size().height,
                        None
                    ));
                    surface_setup_done = true;
                    log::info!("macOS Surface setup complete.");
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
