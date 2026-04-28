use dyxel_render_api::{DeviceHandle, QueueHandle, BackendConfig, RenderBackend, GraphicsRuntime};
use dyxel_render_vello::VelloBackend;
use dyxel_render_vello::runtime::WgpuRuntime;
use vello::{Scene, peniko::{Color, Fill}};

fn main() {
    let mut runtime = WgpuRuntime::new();
    runtime.initialize().expect("Failed to initialize runtime");

    let device = runtime.device().expect("No device");
    let queue = runtime.queue().expect("No queue");

    // Create VelloBackend directly on main thread
    let backend = VelloBackend::new();
    backend.init(
        DeviceHandle::new(device),
        QueueHandle::new(queue),
        BackendConfig { data_dir: String::new() },
    ).expect("Failed to init backend");

    // Wait for renderer to be ready
    loop {
        let guard = backend.renderer.lock().unwrap();
        if guard.is_some() {
            break;
        }
        drop(guard);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    println!("Renderer ready on main thread");

    // Clone device/queue for render thread
    let device_clone = device.clone();
    let queue_clone = queue.clone();
    let backend_clone = std::sync::Arc::new(std::sync::Mutex::new(backend));

    // Spawn a render thread and do rendering from there
    let render_thread = std::thread::spawn(move || {
        let mut scene = Scene::new();
        let rect = vello::kurbo::Rect::from_origin_size((0.0, 0.0), (256.0, 256.0));
        scene.fill(
            Fill::NonZero,
            vello::kurbo::Affine::IDENTITY,
            Color::from_rgba8(255, 0, 0, 255),
            None,
            &rect,
        );

        let target_texture = device_clone.create_texture(
            &wgpu::TextureDescriptor {
                label: Some("Test Target"),
                size: wgpu::Extent3d {
                    width: 256,
                    height: 256,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
        );
        let target_view = target_texture.create_view(
            &wgpu::TextureViewDescriptor::default());

        {
            let backend_guard = backend_clone.lock().unwrap();
            let mut renderer_guard = backend_guard.renderer.lock().unwrap();
            let renderer = renderer_guard.as_mut().unwrap();
            renderer
                .render_to_texture(
                    &device_clone,
                    &queue_clone,
                    &scene,
                    &target_view,
                    &vello::RenderParams {
                        base_color: Color::TRANSPARENT,
                        width: 256,
                        height: 256,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .expect("render_to_texture failed");
        }

        // Read back pixels
        let bytes_per_row = ((256 * 4 + 255) / 256) * 256;
        let readback = device_clone.create_buffer(
            &wgpu::BufferDescriptor {
                label: Some("Readback"),
                size: (bytes_per_row * 256) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            },
        );

        let mut encoder = device_clone.create_command_encoder(
            &Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(256),
                },
            },
            target_texture.size(),
        );
        queue_clone.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        while rx.try_recv().is_err() {
            let _ = device_clone.poll(wgpu::PollType::Poll);
        }

        let data = slice.get_mapped_range();
        let first_pixel = [data[0], data[1], data[2], data[3]];
        let center_offset = (128 * bytes_per_row + 128 * 4) as usize;
        let center_pixel = [data[center_offset], data[center_offset+1], data[center_offset+2], data[center_offset+3]];

        println!("Render thread - First pixel: {:?}", first_pixel);
        println!("Render thread - Center pixel: {:?}", center_pixel);

        let mut non_black = 0;
        for y in 0..256 {
            for x in 0..256 {
                let offset = (y * bytes_per_row + x * 4) as usize;
                if data[offset] > 0 || data[offset+1] > 0 || data[offset+2] > 0 {
                    non_black += 1;
                }
            }
        }
        println!("Render thread - Non-black pixels: {}/65536", non_black);
        non_black
    });

    let non_black = render_thread.join().expect("Render thread panicked");
    assert!(non_black > 1000, "Expected many non-black pixels, got {}", non_black);
    println!("SUCCESS");
}
