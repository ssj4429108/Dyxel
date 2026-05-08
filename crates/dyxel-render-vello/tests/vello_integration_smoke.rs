use dyxel_render_api::{GraphicsRuntime, RenderBackendV2};
use dyxel_render_vello::{backend::VelloDrawingBackend, runtime::WgpuRuntime};

#[test]
fn test_vello_integration_render_to_texture() {
    let mut runtime = WgpuRuntime::new();
    runtime.initialize().expect("Failed to initialize runtime");

    let mut backend = VelloDrawingBackend::new();
    backend
        .initialize(&mut runtime as &mut dyn GraphicsRuntime)
        .expect("Failed to initialize backend");

    // Create a dummy package with a simple scene.
    let package = dyxel_render_api::RenderPackage::new(
        (256, 256),
        Some(0),
        vec![dyxel_render_api::SceneNode {
            id: 0,
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 256.0,
            position_x: 0.0,
            position_y: 0.0,
            content: dyxel_render_api::NodeContent::Rect {
                color: [255, 0, 0, 255],
            },
            border_radius: 0.0,
            opacity: 1.0,
            clip_to_bounds: false,
            shadow: None,
            blur: None,
            children: vec![],
        }],
    );

    // We can't create a real surface without a window, so exercise the backend's
    // render-to-view path directly.
    let device = runtime.device().expect("No device");
    let queue = runtime.queue().expect("No queue");

    // Create a test texture
    let test_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Integration Test Texture"),
        size: wgpu::Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let test_view = test_tex.create_view(&wgpu::TextureViewDescriptor::default());

    // Renderer initialization is intentionally asynchronous for cold-start
    // behavior. Kick it once, wait until ready, then render the package.
    for _ in 0..200 {
        backend
            .vello_backend()
            .render_to_view(
                device,
                queue,
                &test_view,
                wgpu::TextureFormat::Rgba8Unorm,
                &package,
            )
            .expect("Render failed");
        if backend.vello_backend().is_renderer_ready() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        backend.vello_backend().is_renderer_ready(),
        "renderer did not become ready"
    );
    backend
        .vello_backend()
        .render_to_view(
            device,
            queue,
            &test_view,
            wgpu::TextureFormat::Rgba8Unorm,
            &package,
        )
        .expect("Render failed");

    // Read back the test texture
    let bytes_per_row = ((256 * 4 + 255) / 256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Readback"),
        size: (bytes_per_row * 256) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &test_tex,
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
        test_tex.size(),
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    while rx.try_recv().is_err() {
        let _ = device.poll(wgpu::PollType::Poll);
    }

    let data = slice.get_mapped_range();
    let first_pixel = [data[0], data[1], data[2], data[3]];
    println!("Integration test first pixel: {:?}", first_pixel);

    assert!(
        first_pixel[0] > 0 || first_pixel[1] > 0 || first_pixel[2] > 0,
        "Expected non-black pixel, got {:?}",
        first_pixel
    );
}
