use std::sync::Arc;
use winit::{event::*, event_loop::{ControlFlow, EventLoop}, window::WindowBuilder};
use host_core::VelloHost;
use kurbo::Vec2;

fn main() -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    let window = Arc::new(WindowBuilder::new()
        .with_title("VelloView Native (macOS)")
        .with_inner_size(winit::dpi::LogicalSize::new(1024.0, 768.0))
        .build(&event_loop)?);

    let host = VelloHost::new();
    
    // 使用全平台统一的异步 setup
    pollster::block_on(host.setup(
        vello::wgpu::SurfaceTarget::from(window.clone()),
        ".".to_string(), // data_dir
        window.inner_size().width,
        window.inner_size().height,
        None
    ));

    let mut mouse_pos = Vec2::ZERO;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Poll);
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => elwt.exit(),
            Event::WindowEvent { event: WindowEvent::Resized(size) , .. } => {
                host.resize_native(size.width, size.height);
            }
            Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } => {
                mouse_pos = Vec2::new(position.x, position.y);
            }
            Event::WindowEvent { event: WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. }, .. } => {
                host.on_touch(mouse_pos.x as f32, mouse_pos.y as f32);
            }
            Event::AboutToWait => {
                host.tick();
            }
            _ => {}
        }
    })?;

    Ok(())
}
