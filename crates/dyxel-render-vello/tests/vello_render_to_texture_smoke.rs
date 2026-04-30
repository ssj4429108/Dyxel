// Minimal standalone test: verify Vello 0.7.0 render_to_texture works

use vello::{
    peniko::{Color, Fill},
    Renderer, RendererOptions, Scene,
};

#[test]
fn test_vello_render_to_texture() {
    // env_logger::init();

    // Create wgpu instance and adapter
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::from_build_config(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::from_env_or_default(),
    });

    let adapter = pollster::block_on(async {
        instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find adapter")
    });

    println!("Adapter: {:?}", adapter.get_info());

    let (device, queue) = pollster::block_on(async {
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Test Device"),
                required_features: adapter.features()
                    & (wgpu::Features::CLEAR_TEXTURE | wgpu::Features::PIPELINE_CACHE),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .expect("Failed to create device")
    });

    // Create Vello renderer
    let mut renderer = Renderer::new(
        &device,
        RendererOptions {
            antialiasing_support: vello::AaSupport::area_only(),
            pipeline_cache: None,
            num_init_threads: std::num::NonZeroUsize::new(1),
            use_cpu: false,
        },
    )
    .expect("Failed to create Vello renderer");

    // Create a simple scene with a red rectangle
    let mut scene = Scene::new();
    let rect = vello::kurbo::Rect::from_origin_size((0.0, 0.0), (256.0, 256.0));
    scene.fill(
        Fill::NonZero,
        vello::kurbo::Affine::IDENTITY,
        Color::from_rgba8(255, 0, 0, 255),
        None,
        &rect,
    );

    // Create target texture
    let target_texture = device.create_texture(&wgpu::TextureDescriptor {
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
    });
    let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Render
    renderer
        .render_to_texture(
            &device,
            &queue,
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

    // NOTE: Skipping explicit GPU sync to test if ordering is sufficient
    // device.poll(wgpu::PollType::Wait { ... }).unwrap();

    // Read back pixels
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
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    while rx.try_recv().is_err() {
        device.poll(wgpu::PollType::Poll);
    }

    let data = slice.get_mapped_range();
    let first_pixel = [data[0], data[1], data[2], data[3]];
    let center_pixel = {
        let offset = (128 * bytes_per_row + 128 * 4) as usize;
        [
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]
    };

    println!("First pixel: {:?}", first_pixel);
    println!("Center pixel: {:?}", center_pixel);

    // Expect red (255, 0, 0, 255)
    assert_eq!(
        first_pixel,
        [255, 0, 0, 255],
        "Expected red pixel, got {:?}",
        first_pixel
    );
    assert_eq!(
        center_pixel,
        [255, 0, 0, 255],
        "Expected red pixel at center, got {:?}",
        center_pixel
    );

    println!("SUCCESS: render_to_texture works correctly!");
}
