// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_perf::{PerfConfig, PerformanceDiagnostics, PerformanceMonitor, SharedPerfMonitor};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use dyxel_render_api::{
    BackendConfig, DeviceHandle, LifecycleEvent, QueueHandle, RenderBackend, RenderBackendExt,
    RenderContext, RenderResult, SharedMutex, SurfaceHandle, SurfaceState, SurfaceTargetHandle,
    VelloBackendExt,
};
use kurbo::{Affine, Vec2};
use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use vello::wgpu;
use vello::{Renderer, Scene};

mod blur;
mod cache;
mod cold_start;
pub(crate) mod color;
mod coordinates;
mod debug_utils;
mod frame;
mod render_helpers;
mod shadow;
pub(crate) mod text;
use blur::*;
use cache::CachedDraw;
use color::{apply_opacity_to_color, neutral_to_peniko_color};
pub use coordinates::platform_correction;
use frame::TripleBuffer;
use shadow::{
    draw_node_shadow, ShadowCacheEntry, ShadowCacheKey, ShadowCacheRefs, ShadowCacheStats,
};
use text::{draw_prepared_text, GlyphRunCacheEntry, GlyphRunCacheKey, GlyphRunCacheStats};

// Two-stage init is implemented inline with cache header markers

#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "android")]
mod android_native_presenter;
#[cfg(target_os = "android")]
mod android_native_wgpu_ahb;
#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_arch = "wasm32")]
pub mod web;

pub mod backend;
pub mod factory;
pub mod filter_pipeline;
pub mod frame_context;
pub mod minimal_shaders;
pub mod runtime;
pub mod scene_adapter;
pub mod shader_cache;
pub mod staged_init;
pub mod staged_loader;
pub mod texture_pool;
pub mod two_stage_init;

/// Vello render backend implementation
///
/// This is the concrete implementation of RenderBackend using Vello + wgpu
// Type aliases for shared data used in async context
type AsyncShared<T> = std::sync::Arc<std::sync::Mutex<T>>;

/// Frame counter for cache invalidation
pub(crate) static FRAME_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
/// Monotonic frame marker for blur-entry lifetime/reuse across scene builds.
pub(crate) static BLUR_SCENE_FRAME: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Node counter for debugging black screen (limit nodes to find breaking point)
pub(crate) static NODE_COUNTER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Keep normal performance runs from being dominated by Android logcat/macOS
/// unified logging. Per-frame info logs are useful while diagnosing, but they
/// are expensive enough to show up as Scene/Jank noise.
pub(crate) const DIAG_LOG_EVERY_N_FRAMES: u64 = 60;

/// Vello render backend implementation
///
/// This is the concrete implementation of RenderBackend using Vello + wgpu
pub struct VelloBackend {
    pub renderer: AsyncShared<Option<Renderer>>,
    pub blit_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    pub sampler: SharedMutex<Option<wgpu::Sampler>>,
    pub blit_shader: SharedMutex<Option<wgpu::ShaderModule>>,
    pub blit_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    /// Format the blit pipeline was created for; used to detect surface format changes.
    blit_pipeline_format: SharedMutex<Option<wgpu::TextureFormat>>,
    /// Triple buffer for offscreen compositing (managed internally, not per-surface).
    triple_buffer: SharedMutex<Option<TripleBuffer>>,
    // Pipeline for rendering children texture with alpha blending
    children_blit_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    pub pipeline_cache: AsyncShared<Option<wgpu::PipelineCache>>,
    pub cache_path: AsyncShared<Option<String>>,
    pub cache_saved: AtomicBool,
    // Current cache stage: None = no cache, Some(1) = Stage 1, Some(2) = Stage 2
    cache_stage: AsyncShared<Option<u8>>,
    // Deferred initialization - store device info for lazy init
    init_device_info: SharedMutex<Option<(String, Option<wgpu::PipelineCache>, Option<u8>)>>,
    // Performance monitoring
    perf_monitor: SharedPerfMonitor,
    // Detailed diagnostics (optional, for profiling)
    #[allow(dead_code)]
    diagnostics: SharedMutex<Option<PerformanceDiagnostics>>,
    // Performance overlay disabled (was using Editor; TODO: reimplement with PreparedText)
    // Memory optimizer for tiered memory configuration
    memory_optimizer: SharedMutex<dyxel_perf::MemoryOptimizer>,
    // Async initialization state tracking
    is_loading: std::sync::Arc<std::sync::atomic::AtomicBool>,
    // Async loading thread handle (optional - for monitoring)
    #[allow(dead_code)]
    loading_handle: SharedMutex<Option<std::thread::JoinHandle<()>>>,
    // Filter pipeline for blur effects
    filter_pipeline: SharedMutex<Option<filter_pipeline::FilterPipeline>>,
    // Blur composite pipeline for drawing blurred textures
    blur_composite_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    blur_composite_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    blur_composite_uniforms: SharedMutex<Option<wgpu::Buffer>>,
    blur_composite_overlay_uniforms: SharedMutex<Option<wgpu::Buffer>>,
    // Instanced backdrop blur composite path: one bind group + one draw for all blur quads.
    blur_instanced_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    blur_instanced_pipeline_format: SharedMutex<Option<wgpu::TextureFormat>>,
    blur_instanced_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    // Cached bind group for the global backdrop texture + stable per-frame buffers.
    // Invalidated when the backdrop texture, pipeline layout, frame uniform, or instance buffer changes.
    blur_instanced_bind_group: SharedMutex<Option<wgpu::BindGroup>>,
    blur_instance_buffer: SharedMutex<Option<wgpu::Buffer>>,
    blur_instance_capacity: SharedMutex<usize>,
    blur_frame_uniform: SharedMutex<Option<wgpu::Buffer>>,
    // Staging buffer for zero-copy blur uniform updates
    blur_staging_buffer: SharedMutex<Option<wgpu::Buffer>>,
    blur_staging_alignment: SharedMutex<usize>,
    blur_staging_offset: std::sync::atomic::AtomicUsize,
    // Blurred textures to composite (cleared each frame)
    blurred_textures: SharedMutex<Vec<BlurredTextureEntry>>,
    // Full-screen blurred backdrop used by the new backdrop-filter path.
    backdrop_blur: SharedMutex<Option<BackdropBlurTexture>>,
    // Atlas used to batch-composite legacy-correct per-entry blurred textures.
    blur_atlas: SharedMutex<Option<BlurAtlasTexture>>,
    // Raw backdrop atlas used by the atlas-wide blur path. Each visible blur
    // entry is copied into its fixed slot with the same padding as the legacy
    // per-entry texture, then `blur_atlas` receives the blurred result.
    blur_source_atlas: SharedMutex<Option<BlurAtlasTexture>>,
    blur_atlas_wide_active_last_frame: AtomicBool,
    // Texture pool for efficient blur texture reuse
    texture_pool: SharedMutex<Option<texture_pool::SharedTexturePool>>,
    // GPU-local cache storage: node_id -> texture_id lookup table.
    // Runtime decides which nodes to bake (via bake_plans in RenderPackage).
    // Backend only executes bakes and performs read-only lookups during render.
    cached_textures:
        SharedMutex<std::collections::HashMap<u32, dyxel_render_api::raster_cache::TextureId>>,
    // GPU texture pool for raster cache baking
    gpu_texture_pool: SharedMutex<Option<texture_pool::GpuTexturePool>>,
    // Cached shadow textures (geometry+style key -> pre-rendered shadow)
    shadow_cache: SharedMutex<std::collections::HashMap<ShadowCacheKey, ShadowCacheEntry>>,
    // Shadow cache statistics for DIAG logging
    shadow_cache_stats: SharedMutex<ShadowCacheStats>,
    // Per-frame cap on shadow cache misses to avoid GPU submit spikes
    shadow_cache_misses_this_frame: AtomicU64,
    // Monotonically-incremented ID to detect renderer replacement (Stage 1 -> Stage 2).
    // Shadow cache entries are tied to a specific renderer instance.
    renderer_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    // Last seen renderer_id; mismatch means renderer was replaced and cache must be cleared.
    shadow_cache_renderer_id: std::sync::atomic::AtomicU64,
    // Frame timing from pacer (for DIAG logging)
    pacer_wait_ms: SharedMutex<f64>,
    frame_interval_ms: SharedMutex<f64>,
    // Cached glyph runs (font+text signature -> pre-built vello glyphs)
    glyph_run_cache: SharedMutex<std::collections::HashMap<GlyphRunCacheKey, GlyphRunCacheEntry>>,
    // Glyph run cache statistics for DIAG logging
    glyph_run_cache_stats: SharedMutex<GlyphRunCacheStats>,
    // Frame performance stats from scheduler (for DIAG logging)
    frame_perf_stats: SharedMutex<dyxel_perf::FramePerformanceStats>,
}

const BLIT_SHADER_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blit.spv"));

impl VelloBackend {
    pub fn new() -> Self {
        Self::with_perf_config(PerfConfig::default())
    }

    pub fn with_perf_config(perf_config: PerfConfig) -> Self {
        // Initialize memory optimizer with tiered configuration
        let memory_optimizer = dyxel_perf::MemoryOptimizer::new();
        log::info!(
            "[Memory] VelloBackend: Device tier detected: {:?}",
            memory_optimizer.tier()
        );

        Self {
            renderer: AsyncShared::new(std::sync::Mutex::new(None)),
            blit_bind_group_layout: SharedMutex::new(None),
            sampler: SharedMutex::new(None),
            blit_shader: SharedMutex::new(None),
            blit_pipeline: SharedMutex::new(None),
            blit_pipeline_format: SharedMutex::new(None),
            triple_buffer: SharedMutex::new(None),
            children_blit_pipeline: SharedMutex::new(None),
            pipeline_cache: AsyncShared::new(std::sync::Mutex::new(None)),
            cache_path: AsyncShared::new(std::sync::Mutex::new(None)),
            cache_saved: AtomicBool::new(false),
            cache_stage: AsyncShared::new(std::sync::Mutex::new(None)),
            init_device_info: SharedMutex::new(None),
            perf_monitor: std::sync::Arc::new(std::sync::Mutex::new(PerformanceMonitor::new(
                perf_config,
            ))),
            diagnostics: SharedMutex::new(Some(PerformanceDiagnostics::new(120))),
            memory_optimizer: SharedMutex::new(memory_optimizer),
            is_loading: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            loading_handle: SharedMutex::new(None),
            filter_pipeline: SharedMutex::new(None),
            blur_composite_pipeline: SharedMutex::new(None),
            blur_composite_bind_group_layout: SharedMutex::new(None),
            blur_composite_uniforms: SharedMutex::new(None),
            blur_composite_overlay_uniforms: SharedMutex::new(None),
            blur_instanced_pipeline: SharedMutex::new(None),
            blur_instanced_pipeline_format: SharedMutex::new(None),
            blur_instanced_bind_group_layout: SharedMutex::new(None),
            blur_instanced_bind_group: SharedMutex::new(None),
            blur_instance_buffer: SharedMutex::new(None),
            blur_instance_capacity: SharedMutex::new(0),
            blur_frame_uniform: SharedMutex::new(None),
            blur_staging_buffer: SharedMutex::new(None),
            blur_staging_alignment: SharedMutex::new(256),
            blur_staging_offset: std::sync::atomic::AtomicUsize::new(0),
            blurred_textures: SharedMutex::new(Vec::new()),
            backdrop_blur: SharedMutex::new(None),
            blur_atlas: SharedMutex::new(None),
            blur_source_atlas: SharedMutex::new(None),
            blur_atlas_wide_active_last_frame: AtomicBool::new(false),
            texture_pool: SharedMutex::new(None),
            cached_textures: SharedMutex::new(std::collections::HashMap::new()),
            gpu_texture_pool: SharedMutex::new(None),
            shadow_cache: SharedMutex::new(std::collections::HashMap::new()),
            shadow_cache_stats: SharedMutex::new(ShadowCacheStats::default()),
            shadow_cache_misses_this_frame: AtomicU64::new(0),
            renderer_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
            shadow_cache_renderer_id: std::sync::atomic::AtomicU64::new(0),
            glyph_run_cache: SharedMutex::new(std::collections::HashMap::new()),
            glyph_run_cache_stats: SharedMutex::new(GlyphRunCacheStats::default()),
            pacer_wait_ms: SharedMutex::new(0.0),
            frame_interval_ms: SharedMutex::new(0.0),
            frame_perf_stats: SharedMutex::new(dyxel_perf::FramePerformanceStats::default()),
        }
    }

    /// Enable performance overlay
    pub fn enable_perf_overlay(&self) {
        self.perf_monitor.lock().unwrap().toggle_overlay();
    }

    /// Disable performance overlay
    pub fn disable_perf_overlay(&self) {
        let mut monitor = self.perf_monitor.lock().unwrap();
        if monitor.should_show_overlay() {
            monitor.toggle_overlay();
        }
    }

    fn render_internal_impl(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_view: &wgpu::TextureView,
        surface_format: wgpu::TextureFormat,
        package: &dyxel_render_api::RenderPackage,
    ) -> anyhow::Result<Option<vello::wgpu::SubmissionIndex>> {
        // Derive render inputs from the immutable package (no runtime objects)
        let node_map: std::collections::HashMap<u32, &dyxel_render_api::SceneNode> =
            package.nodes.iter().map(|n| (n.id, n)).collect();
        let rid = package.root_id;
        let w = package.viewport.0;
        let h = package.viewport.1;
        let diag_seq = FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
        let diag_log_this_frame = diag_seq % DIAG_LOG_EVERY_N_FRAMES == 0;
        if diag_log_this_frame {
            log::info!(
                "[DIAG] Package: nodes={} root_id={:?} viewport={}x{}",
                package.nodes.len(),
                rid,
                w,
                h
            );
            if let Some(first) = package.nodes.first() {
                log::info!(
                    "[DIAG] First node: id={} xy=({:.1},{:.1}) wh=({:.1},{:.1}) opacity={:.2} content={:?}",
                    first.id,
                    first.x,
                    first.y,
                    first.width,
                    first.height,
                    first.opacity,
                    std::mem::discriminant(&first.content)
                );
            }
        }

        // Backend-internal frame housekeeping (was prepare_internal)
        self.collect_returned_textures_and_reset_blur_staging();

        // Detailed frame timing for diagnostics
        let frame_start = std::time::Instant::now();
        let mut stage_timer = dyxel_perf::FrameTimer::new();

        // Async initialization: start background compilation without blocking
        self.ensure_renderer_initialized_async(device, queue);
        stage_timer.mark("init_check");

        // Check if renderer is ready
        let mut renderer_lock = self.renderer.lock().unwrap();
        let renderer = match renderer_lock.as_mut() {
            Some(r) => {
                if diag_log_this_frame {
                    log::info!("[DIAG] Renderer ready");
                }
                r
            }
            None => {
                // Renderer not ready yet - clear surface and return
                log::info!("[DIAG] Renderer not ready, clearing surface");
                // This keeps the main loop at 60fps while shader compiles in background
                drop(renderer_lock); // Release lock before calling clear_surface
                return self.clear_surface(device, queue, target_view);
            }
        };

        // Begin frame timing for performance monitoring
        {
            let monitor = self.perf_monitor.lock().unwrap();
            monitor.begin_frame();
        }
        stage_timer.mark("perf_start");

        if w == 0 || h == 0 {
            return Ok(None);
        }

        // Reset per-frame shadow cache miss counter
        self.reset_shadow_frame_budget();

        // Shadow cache LRU eviction: remove entries unused for 300 frames (5s @ 60fps)
        self.evict_stale_shadow_cache_entries(renderer);

        // Detect renderer replacement (e.g. Stage 1 -> Stage 2 cold-start upgrade).
        // When renderer is replaced, image_overrides are lost; stale cache entries
        // would trigger "invalid empty image" panic on draw_image.
        self.sync_shadow_cache_renderer_id(renderer);

        // Glyph run cache LRU eviction: remove entries unused for 300 frames
        self.evict_stale_glyph_runs();

        let mut scene = Scene::new();
        let mut cached_draws: Vec<CachedDraw> = Vec::new();

        stage_timer.mark("state_lock");
        self.build_main_scene(
            rid,
            &node_map,
            &mut scene,
            &mut cached_draws,
            h,
            device,
            queue,
            renderer,
        );
        stage_timer.mark("scene_build");

        stage_timer.mark("bake_start");
        self.execute_raster_cache_plans(package, &node_map, device, queue, renderer);
        stage_timer.mark("bake_done");

        self.ensure_triple_buffer(device, queue, w, h);
        let mut triple_buffer = self.triple_buffer.lock().unwrap();
        let tb = triple_buffer.as_mut().unwrap();
        let aa_config = self.current_aa_config();
        self.render_main_scene_to_texture(
            device,
            queue,
            renderer,
            &scene,
            &tb.current().view,
            w,
            h,
            aa_config,
            diag_log_this_frame,
        )?;
        stage_timer.mark("gpu_render");

        // OPTIMIZATION: Removed blocking wait. GPU commands are naturally ordered by submission.
        // The copy operations in Pass 2 will execute after the scene render completes.
        // This allows CPU to continue preparing blur commands while GPU renders the scene.

        #[cfg(not(target_arch = "wasm32"))]
        self.debug_save_pass1_scene_texture(device, queue, &tb.current().texture);

        // === PASS 2: Process blur textures from scene ===
        let mut post_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Frame Encoder"),
        });
        let (atlas_wide_blur_valid_this_frame, atlas_wide_source_copies_this_frame) = self
            .process_blur_pass2(
                device,
                &mut post_enc,
                &tb.current().texture,
                w,
                h,
                &mut stage_timer,
            );
        let has_blur = !self.blurred_textures.lock().unwrap().is_empty();

        stage_timer.mark("pass3_start");

        // === PASS 3: Render deferred children to per-entry local textures ===
        // Only run when there are actual blur entries with deferred children.
        if has_blur {
            let mut blurred_textures = self.blurred_textures.lock().unwrap();
            #[allow(unused_variables)]
            let rendered_indices = render_pass3_children(
                &mut blurred_textures,
                &node_map,
                device,
                queue,
                renderer,
                aa_config,
                &self.glyph_run_cache,
                &self.glyph_run_cache_stats,
                w,
                h,
            );
            #[cfg(not(target_arch = "wasm32"))]
            if self.debug_frames_enabled() {
                let frame_num = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let debug_dir = self.debug_output_dir();
                for &idx in &rendered_indices {
                    if let Some(entry) = blurred_textures.get(idx) {
                        let path = debug_dir.join(format!(
                            "frame_{:06}_pass3_children_view_{}.png",
                            frame_num % 1000,
                            entry.view_id
                        ));
                        if let Some(ref tex) = entry.children_texture {
                            self.save_texture_to_png(device, queue, tex, path.to_str().unwrap());
                        }
                    }
                }
            }
        }
        stage_timer.mark("pass3_done");

        // === PASS 3.5: Pack blur textures into atlas for instanced composite ===
        let (atlas_bind_group, atlas_instance_count, atlas_enabled_this_frame) = self
            .pack_blur_atlas_pass(
                device,
                queue,
                &mut post_enc,
                surface_format,
                atlas_wide_blur_valid_this_frame,
                atlas_wide_source_copies_this_frame,
                w,
                h,
            );

        // Surface texture was already acquired by GraphicsRuntime::begin_frame().
        // No texture wait happens inside the backend render path.
        stage_timer.mark("surface_ready");

        // === PASS 4: Final Blit ===
        // Determine render target (capture texture for debug, else surface directly)
        #[cfg(not(target_arch = "wasm32"))]
        let (capture_texture, debug_frame_num, render_target_view) =
            self.create_debug_capture(device, surface_format, target_view, w, h);
        #[cfg(target_arch = "wasm32")]
        let render_target_view = target_view.clone();

        #[allow(unused)]
        let mut _had_blur_textures = false;
        {
            // Ensure blit pipeline matches the surface format (e.g. Bgra8Unorm on macOS)
            self.ensure_blit_pipeline(device, surface_format);

            let mut rp = post_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Vello Blit Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &render_target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let blit_pipeline_guard = self.blit_pipeline.lock().unwrap();
            let blit_pipeline = blit_pipeline_guard.as_ref().unwrap();
            rp.set_pipeline(blit_pipeline);
            rp.set_bind_group(0, &tb.current().bind_group, &[]);
            rp.draw(0..3, 0..1);
            drop(blit_pipeline_guard);

            // Draw blurred textures using composite pipeline (skip when neither blur nor cache draws exist)
            if has_blur || !cached_draws.is_empty() {
                let mut blurred_textures = self.blurred_textures.lock().unwrap();

                let needs_pipeline = self.blur_composite_pipeline.lock().unwrap().is_none();
                if needs_pipeline {
                    self.create_blur_composite_pipeline(device, surface_format);
                }

                let blur_pipeline = self.blur_composite_pipeline.lock().unwrap();
                let blur_bg_layout = self.blur_composite_bind_group_layout.lock().unwrap();
                let uniform_buffer = self.blur_composite_uniforms.lock().unwrap();
                let overlay_uniform_buffer = self.blur_composite_overlay_uniforms.lock().unwrap();

                if let (Some(pipeline), Some(layout), _, _) = (
                    blur_pipeline.as_ref(),
                    blur_bg_layout.as_ref(),
                    uniform_buffer.as_ref(),
                    overlay_uniform_buffer.as_ref(),
                ) {
                    let sampler_guard = self.sampler.lock().unwrap();
                    let sampler = sampler_guard
                        .as_ref()
                        .expect("Sampler should be initialized");
                    let staging_guard = self.blur_staging_buffer.lock().unwrap();
                    let staging = staging_guard
                        .as_ref()
                        .expect("blur staging buffer not initialized");
                    let alignment = *self.blur_staging_alignment.lock().unwrap();
                    let gpu_pool_guard = self.gpu_texture_pool.lock().unwrap();
                    let atlas_pipeline_guard = self.blur_instanced_pipeline.lock().unwrap();
                    let backdrop_guard = self.backdrop_blur.lock().unwrap();
                    let backdrop_view = if USE_FULL_FRAME_BACKDROP_BLUR {
                        backdrop_guard.as_ref().map(|b| &b.view)
                    } else {
                        None
                    };

                    let res = BlurCompositeResources {
                        pipeline,
                        layout,
                        sampler,
                        staging_buffer: staging,
                        staging_alignment: alignment,
                        staging_offset: &self.blur_staging_offset,
                        gpu_texture_pool: gpu_pool_guard.as_ref(),
                        atlas_pipeline: atlas_pipeline_guard.as_ref(),
                        atlas_bind_group: atlas_bind_group.as_ref(),
                        atlas_instance_count,
                        atlas_enabled: atlas_enabled_this_frame,
                        backdrop_view,
                    };
                    _had_blur_textures = composite_blur_pass4(
                        &mut rp,
                        &mut blurred_textures,
                        &cached_draws,
                        &res,
                        device,
                        queue,
                        w,
                        h,
                    );
                }
            }
        }

        // If using capture texture, blit it to surface before present (same encoder)
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(ref capture_tex) = capture_texture {
            self.ensure_blit_pipeline(device, surface_format);
            let capture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Capture Blit Bind Group"),
                layout: self
                    .blit_bind_group_layout
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap(),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &capture_tex.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(
                            self.sampler.lock().unwrap().as_ref().unwrap(),
                        ),
                    },
                ],
            });

            {
                let mut rp = post_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Capture Blit Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                let blit_pipeline_guard = self.blit_pipeline.lock().unwrap();
                let blit_pipeline = blit_pipeline_guard.as_ref().unwrap();
                rp.set_pipeline(blit_pipeline);
                rp.set_bind_group(0, &capture_bind_group, &[]);
                rp.draw(0..3, 0..1);
            }
        }

        // Single submit for all post-Vello GPU work
        let submission_index = queue.submit(Some(post_enc.finish()));
        stage_timer.mark("blit_submit");

        // Debug: Save composite frame when we have blur textures
        #[cfg(not(target_arch = "wasm32"))]
        {
            log::debug!("[Debug] Checking had_blur_textures = {}", _had_blur_textures);
            if _had_blur_textures && self.debug_frames_enabled() {
                if let Some(capture_tex) = &capture_texture {
                    let debug_dir = self.debug_output_dir();
                    let frame_num = debug_frame_num.unwrap_or(0);
                    let capture_path =
                        debug_dir.join(format!("frame_{:06}_pass0_composite.png", frame_num));
                    log::debug!(
                        "[DebugSave disabled] Would save composite frame to {:?}",
                        capture_path
                    );
                    self.save_texture_to_png(
                        device,
                        queue,
                        capture_tex,
                        capture_path.to_str().unwrap(),
                    );
                }
            }
        }

        // Present is handled by GraphicsRuntime::end_frame (outside backend).
        stage_timer.mark("render_return");

        // After first successful render, save the pipeline cache
        static FIRST_RENDER_DONE: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if !FIRST_RENDER_DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
            log::info!("[ColdStart] First render completed, saving pipeline cache");
            self.save_cache();
        }

        // Log detailed frame timing and performance stats for diagnostics
        self.log_frame_diagnostics(&stage_timer, frame_start);

        Ok(Some(submission_index))
    }

    /// Render a package using a pre-acquired surface texture (double-layer API entry point).
    ///
    /// The caller (e.g. `GraphicsRuntime::end_frame`) is responsible for presenting the surface texture.
    pub fn render_with_surface_texture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_texture: &wgpu::SurfaceTexture,
        surface_format: wgpu::TextureFormat,
        package: &dyxel_render_api::RenderPackage,
    ) -> anyhow::Result<Option<vello::wgpu::SubmissionIndex>> {
        let target_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_internal_impl(device, queue, &target_view, surface_format, package)
    }

    /// Render a package into a caller-owned texture view.
    ///
    /// This is used by the macOS offscreen-first architecture: the runtime
    /// renders the full Vello/composite result into an offscreen target and
    /// acquires the actual surface drawable only in `end_frame`.
    pub fn render_to_view(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_view: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        package: &dyxel_render_api::RenderPackage,
    ) -> anyhow::Result<Option<vello::wgpu::SubmissionIndex>> {
        self.render_internal_impl(device, queue, target_view, target_format, package)
    }

    /// Public entry point: acquires cached_textures lock once, then delegates to internal recursive renderer.
    fn render_node_recursive_with_transform(
        &self,
        id: u32,
        nodes: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
        scene: &mut Scene,
        parent_pos: Vec2,
        transform: Affine,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut vello::Renderer,
        filter_pipeline: Option<&crate::filter_pipeline::FilterPipeline>,
        blurred_textures: &mut Vec<BlurredTextureEntry>,
        cached_draws: &mut Vec<CachedDraw>,
        in_blur_subtree: bool,
        blur_scene_frame: u64,
    ) {
        let cache_guard = self.cached_textures.lock().unwrap();
        self.render_node_recursive_internal(
            id,
            nodes,
            scene,
            parent_pos,
            transform,
            device,
            queue,
            renderer,
            filter_pipeline,
            blurred_textures,
            &*cache_guard,
            cached_draws,
            in_blur_subtree,
            blur_scene_frame,
        );
    }
}

// render_with_blur moved to blur/entry.rs
impl VelloBackend {
    /// Render a node with layer effects (alpha, blur, shadow, clip)
    /// Following Xilem's pattern: shadow -> content -> children
    fn render_node_recursive_internal(
        &self,
        id: u32,
        nodes: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
        scene: &mut Scene,
        parent_pos: Vec2,
        transform: Affine,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut vello::Renderer,
        filter_pipeline: Option<&crate::filter_pipeline::FilterPipeline>,
        blurred_textures: &mut Vec<BlurredTextureEntry>,
        cached_textures: &std::collections::HashMap<u32, dyxel_render_api::raster_cache::TextureId>,
        cached_draws: &mut Vec<CachedDraw>,
        in_blur_subtree: bool,
        blur_scene_frame: u64,
    ) {
        use kurbo::{Affine, Rect as KRect, RoundedRect};
        use vello::peniko::{BlendMode as PenikoBlendMode, Compose, Fill, Mix};

        if let Some(node) = nodes.get(&id).copied() {
            let taffy_x = node.x as f64;
            let taffy_y = node.y as f64;
            let node_width = node.width as f64;
            let node_height = node.height as f64;
            let pos_offset = Vec2::new(node.position_x as f64, node.position_y as f64);

            // When position is set, treat it as absolute coordinates within the parent
            // (ignoring Taffy layout position) rather than an offset on top of layout.
            let is_absolute = node.position_x != 0.0 || node.position_y != 0.0;
            let global_pos = if is_absolute {
                parent_pos + pos_offset
            } else {
                parent_pos + Vec2::new(taffy_x, taffy_y)
            };

            // Build local transform for this node
            let local_transform = transform * Affine::translate((global_pos.x, global_pos.y));

            // Determine if we need layer effects
            let _has_shadow = node.shadow.is_some();
            let has_blur = node.blur.is_some();
            let has_children = !node.children.is_empty();
            // OPTIMIZATION: Leaf nodes with only opacity don't need a layer.
            // We can apply opacity directly to the fill color, avoiding costly
            // per-tile clip commands that blow up the PTCL buffer.
            let needs_layer_for_opacity = node.opacity < 1.0 && has_children;
            let needs_layer = needs_layer_for_opacity || node.clip_to_bounds || has_blur;

            // === Raster Cache Check ===
            // Conservative eligibility: only nodes fully outside any blur subtree.
            // Backend performs read-only lookup; Runtime decides which nodes to bake.
            let node_in_blur_subtree = in_blur_subtree || has_blur;
            if !node_in_blur_subtree {
                if let Some(&texture_id) = cached_textures.get(&id) {
                    cached_draws.push(CachedDraw {
                        texture_id: texture_pool::TextureId(texture_id.0),
                        transform: Affine::translate((global_pos.x, global_pos.y)),
                        width: node_width as f32,
                        height: node_height as f32,
                    });
                    return;
                }
            }

            // NOTE: When blur is enabled, we skip layer creation here because:
            // 1. The node's background should NOT be drawn to the main scene
            // 2. Blur effect handles opacity and compositing separately
            let needs_layer_without_blur = needs_layer && !has_blur;

            // Debug: Log blur node info
            if has_blur {
                log::debug!(
                    "[Debug] Blur node id={} blur_radius={} opacity={}",
                    id,
                    node.blur.as_ref().map(|b| b.blur_radius).unwrap_or(0.0),
                    node.opacity
                );
                log::debug!(
                    "[Debug] Position: taffy=({:.1},{:.1}) global=({:.1},{:.1}) size={:.1}x{:.1}",
                    taffy_x,
                    taffy_y,
                    global_pos.x,
                    global_pos.y,
                    node_width,
                    node_height
                );
                log::debug!(
                    "[Debug] BEFORE check: id={} needs_layer={} has_blur={} needs_layer_without_blur={}",
                    id,
                    needs_layer,
                    has_blur,
                    needs_layer_without_blur
                );
            }

            // === Step 1: Draw Shadow (if any, using blur) ===
            // Xilem pattern: Draw shadow first, then content on top
            // NOTE: When blur is enabled, skip shadow in Pass 1. Shadow will be handled
            // by the blur compositing pipeline to avoid double-rendering.
            log::debug!(
                "[ShadowCheck] id={} has_shadow={} has_blur={}",
                id,
                node.shadow.is_some(),
                has_blur
            );
            if let Some(ref shadow) = node.shadow {
                if !has_blur {
                    draw_node_shadow(
                        id,
                        shadow,
                        scene,
                        local_transform,
                        node_width,
                        node_height,
                        node.border_radius as f64,
                        device,
                        queue,
                        renderer,
                        ShadowCacheRefs {
                            cache: &self.shadow_cache,
                            stats: &self.shadow_cache_stats,
                            misses_this_frame: &self.shadow_cache_misses_this_frame,
                        },
                    );
                }
            }

            // === Step 2: Push Layer (if needed for alpha/blur/clip) ===
            // NOTE: When blur is enabled, we skip layer creation here because:
            // 1. The node's background should NOT be drawn to the main scene
            // 2. Blur effect handles opacity and compositing separately

            log::debug!(
                "[LayerCheck] id={} needs_layer={} clip_to_bounds={} opacity={} border_radius={}",
                id,
                needs_layer_without_blur,
                node.clip_to_bounds,
                node.opacity,
                node.border_radius
            );
            if needs_layer_without_blur {
                // Convert opacity to layer alpha
                let alpha = node.opacity.clamp(0.0, 1.0);

                // Default blend mode (Normal)
                let blend = PenikoBlendMode::new(Mix::Normal, Compose::SrcOver);

                // Use node's bounds for the layer shape to avoid full-screen clip bloat.
                // If clip_to_bounds is enabled, we clip exactly to the node bounds.
                // Otherwise we still use node bounds (not infinite rect) for performance.
                if node.border_radius > 0.0 {
                    let clip_rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
                    let rounded_clip = RoundedRect::from_rect(clip_rect, node.border_radius as f64);
                    scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &rounded_clip);
                } else {
                    let clip_rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
                    scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &clip_rect);
                }
            }

            // === Step 3: Handle Blur Effect ===
            // If blur is enabled, render to offscreen texture and apply blur
            let blur_applied = if let Some(ref blur) = node.blur {
                if filter_pipeline.is_some() {
                    render_with_blur(
                        blur,
                        id,
                        nodes,
                        scene,
                        local_transform,
                        global_pos,
                        device,
                        queue,
                        renderer,
                        filter_pipeline.unwrap(),
                        node_width,
                        node_height,
                        needs_layer,
                        blurred_textures,
                        blur_scene_frame,
                    )
                } else {
                    false
                }
            } else {
                false
            };

            // === Step 4: Draw Node Content ===
            // Skip normal drawing if blur was applied (blur texture will be drawn in blit pass)
            // Opacity is applied either by the layer (Step 2) or baked into content color here.
            // If a layer was pushed for opacity, we must NOT double-apply it to content.
            let direct_opacity = if needs_layer_without_blur {
                1.0
            } else {
                node.opacity
            };
            if !blur_applied {
                match node.content {
                    dyxel_render_api::NodeContent::Text(ref payload) => {
                        draw_prepared_text(
                            scene,
                            payload,
                            local_transform,
                            &self.glyph_run_cache,
                            &self.glyph_run_cache_stats,
                            direct_opacity,
                        );
                    }
                    dyxel_render_api::NodeContent::Rect { color } => {
                        let rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
                        // Apply opacity directly only when no layer is handling it
                        let effective_color = if direct_opacity < 1.0 {
                            apply_opacity_to_color(color, direct_opacity)
                        } else {
                            color
                        };
                        let pcolor = neutral_to_peniko_color(effective_color);

                        // Debug: Log fill operations for non-text nodes
                        log::debug!(
                            "[DebugFill] id={} color={:?} size={}x{} transform={:?}",
                            id,
                            color,
                            node_width,
                            node_height,
                            local_transform
                        );

                        if node.border_radius > 0.0 {
                            let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                            scene.fill(Fill::NonZero, local_transform, pcolor, None, &rounded);
                        } else {
                            scene.fill(Fill::NonZero, local_transform, pcolor, None, &rect);
                        }
                    }
                }
            }

            // === Step 5: Recursively render children ===
            // For blur views: skip children in Pass 1, they will be rendered to
            // a separate texture in Pass 3 and composited on top of blur in blit pass.
            // For non-blur views: render children normally.
            // DEBUG: Log children traversal
            if !node.children.is_empty() {
                log::debug!(
                    "[DebugChildren] id={} has {} children: {:?}",
                    id,
                    node.children.len(),
                    node.children
                );
            }
            if !blur_applied {
                for &child_id in &node.children {
                    self.render_node_recursive_internal(
                        child_id,
                        nodes,
                        scene,
                        global_pos,
                        transform,
                        device,
                        queue,
                        renderer,
                        filter_pipeline,
                        blurred_textures,
                        cached_textures,
                        cached_draws,
                        node_in_blur_subtree,
                        blur_scene_frame,
                    );
                }
            }

            // === Step 6: Pop Layer (if pushed) ===
            // Only pop layer if we pushed it (when blur is NOT enabled)
            if needs_layer_without_blur {
                scene.pop_layer();
            }
        }
    }
}

impl RenderBackend for VelloBackend {
    fn init(
        &self,
        device: DeviceHandle,
        _queue: QueueHandle,
        config: BackendConfig,
    ) -> RenderResult {
        let init_start = std::time::Instant::now();

        #[cfg(target_os = "android")]
        log::info!("[Android-Perf] VelloBackend::init started - Performance monitoring enabled");

        // Convert DeviceHandle to wgpu::Device reference
        let device = unsafe { &*device.as_ptr::<wgpu::Device>() };

        // Try using pre-compiled SPIR-V, fall back to WGSL if it fails
        let blit_shader = if cfg!(target_os = "android") {
            let spv_words: Vec<u32> = BLIT_SHADER_SPV
                .chunks_exact(4)
                .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();

            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blit Shader (SPIR-V)"),
                source: wgpu::ShaderSource::SpirV(std::borrow::Cow::Owned(spv_words)),
            })
        } else {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blit Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
            })
        };

        let blit_bl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let (cache_path, pipeline_cache, cache_stage) =
            cold_start::load_pipeline_cache(device, &config.data_dir);

        *self.blit_bind_group_layout.lock().unwrap() = Some(blit_bl);
        *self.sampler.lock().unwrap() = Some(sampler);
        *self.blit_shader.lock().unwrap() = Some(blit_shader);
        *self.pipeline_cache.lock().unwrap() = pipeline_cache.clone();
        *self.cache_path.lock().unwrap() = Some(cache_path.clone());
        *self.cache_stage.lock().unwrap() = cache_stage;

        // Prewarm blit pipeline
        self.prewarm_pipelines(device, wgpu::TextureFormat::Rgba8Unorm);

        // Initialize filter pipeline for blur effects
        let device_arc = std::sync::Arc::new(device.clone());
        let queue_arc = std::sync::Arc::new(unsafe { &*_queue.as_ptr::<wgpu::Queue>() }.clone());
        match filter_pipeline::FilterPipeline::new(device_arc, queue_arc) {
            Ok(pipeline) => {
                *self.filter_pipeline.lock().unwrap() = Some(pipeline);
                log::debug!("[Blur] Filter pipeline initialized successfully");
            }
            Err(e) => {
                log::warn!("[Blur] Failed to initialize filter pipeline: {}", e);
                // Continue without blur support
            }
        }

        // Note: Blur composite pipeline is created lazily on first use
        // with the correct surface format to avoid format mismatch

        // Initialize texture pool for efficient blur texture reuse
        {
            let device_arc = Arc::new(device.clone());
            let pool = texture_pool::SharedTexturePool::new(
                device_arc.clone(),
                texture_pool::TexturePoolConfig::default(),
            );
            *self.texture_pool.lock().unwrap() = Some(pool);
            let gpu_pool = texture_pool::GpuTexturePool::new(
                device_arc,
                texture_pool::TexturePoolConfig::default(),
            );
            *self.gpu_texture_pool.lock().unwrap() = Some(gpu_pool);
            log::info!("[TexturePool] Initialized blur texture pool");
        }

        // Raster cache initialization has moved to Runtime.

        // Store info for deferred renderer initialization (includes cache stage)
        *self.init_device_info.lock().unwrap() = Some((cache_path, pipeline_cache, cache_stage));

        // Eagerly start renderer initialization in background so first frame isn't black
        let queue_ref = unsafe { &*_queue.as_ptr::<wgpu::Queue>() };
        self.ensure_renderer_initialized_async(device, queue_ref);

        // Initialize memory optimizer
        {
            let memory_optimizer = self.memory_optimizer.lock().unwrap();
            memory_optimizer.initialize();
            log::info!(
                "[Memory] Initialized memory optimizer for tier: {:?}",
                memory_optimizer.tier()
            );
        }

        log::info!(
            "[Perf] VelloBackend::init: Total time {:?} (Renderer deferred)",
            init_start.elapsed()
        );
        Ok(())
    }

    fn create_surface_state(
        &self,
        context: &mut RenderContext,
        target: Option<SurfaceTargetHandle>,
        surface: Option<SurfaceHandle>,
        _surface_ptr: u64,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Box<dyn SurfaceState>> {
        log::info!(
            "VelloBackend: create_surface_state START - size: {}x{}, has_precreated_surface: {}",
            width,
            height,
            surface.is_some()
        );

        // Downcast RenderContext to vello::util::RenderContext
        let v_ctx = context
            .downcast_mut::<vello::util::RenderContext>()
            .ok_or_else(|| anyhow::anyhow!("RenderContext is not a Vello RenderContext"))?;

        // Select present mode
        #[cfg(target_os = "android")]
        let present_mode = {
            log::info!("VelloBackend: Using Mailbox mode (low latency, VSync-like but faster)");
            wgpu::PresentMode::Mailbox
        };

        #[cfg(not(target_os = "android"))]
        let present_mode = {
            log::info!("VelloBackend: Using Immediate mode (VSync disabled)");
            wgpu::PresentMode::Immediate
        };

        let v_surface = if let Some(s) = surface {
            log::info!(
                "VelloBackend: Using pre-created surface (present_mode: {:?})",
                present_mode
            );
            let wgpu_surface = s
                .into_inner::<wgpu::Surface<'static>>()
                .ok_or_else(|| anyhow::anyhow!("SurfaceHandle is not a wgpu::Surface"))?;
            pollster::block_on(v_ctx.create_render_surface(
                wgpu_surface,
                width,
                height,
                present_mode,
            ))
            .map_err(|e| anyhow::anyhow!("Failed to create render surface: {:?}", e))?
        } else if let Some(t) = target {
            log::info!(
                "VelloBackend: Creating surface from target (present_mode: {:?})",
                present_mode
            );
            let wgpu_target = t
                .into_inner::<wgpu::SurfaceTarget<'static>>()
                .ok_or_else(|| {
                    anyhow::anyhow!("SurfaceTargetHandle is not a wgpu::SurfaceTarget")
                })?;
            pollster::block_on(v_ctx.create_surface(wgpu_target, width, height, present_mode))
                .map_err(|e| anyhow::anyhow!("Failed to create surface: {:?}", e))?
        } else {
            return Err(anyhow::anyhow!("Either target or surface must be provided"));
        };

        log::info!(
            "VelloBackend: Surface created, format: {:?}, dev_id: {}",
            v_surface.config.format,
            v_surface.dev_id
        );

        let blit_layout_lock = self.blit_bind_group_layout.lock().unwrap();
        let blit_shader_lock = self.blit_shader.lock().unwrap();

        let device = &v_ctx.devices[v_surface.dev_id].device;

        let bl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[blit_layout_lock.as_ref().unwrap()],
            push_constant_ranges: &[],
        });

        let blit_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&bl),
            vertex: wgpu::VertexState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: v_surface.config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: self.pipeline_cache.lock().unwrap().as_ref(),
        });

        log::info!("VelloBackend: Blit pipeline created successfully");

        // Create children blit pipeline with alpha blending
        let children_blit_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Children Blit Pipeline"),
            layout: Some(&bl),
            vertex: wgpu::VertexState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: v_surface.config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: self.pipeline_cache.lock().unwrap().as_ref(),
        });
        *self.children_blit_pipeline.lock().unwrap() = Some(children_blit_p);
        *self.blit_pipeline.lock().unwrap() = Some(blit_p);

        #[cfg(target_os = "macos")]
        {
            log::info!("VelloBackend: Creating MacVelloSurfaceState");
            return Ok(Box::new(mac::MacVelloSurfaceState { surface: v_surface }));
        }

        #[cfg(target_os = "android")]
        {
            log::info!("VelloBackend: Creating AndroidVelloSurfaceState");
            return Ok(Box::new(android::AndroidVelloSurfaceState {
                surface: v_surface,
            }));
        }

        #[cfg(target_arch = "wasm32")]
        {
            log::info!("VelloBackend: Creating WebVelloSurfaceState");
            return Ok(Box::new(web::WebVelloSurfaceState { surface: v_surface }));
        }

        #[cfg(all(
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_arch = "wasm32")
        ))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn set_frame_timing(&self, pacer_wait_ms: f64, frame_interval_ms: f64) {
        *self.pacer_wait_ms.lock().unwrap() = pacer_wait_ms;
        *self.frame_interval_ms.lock().unwrap() = frame_interval_ms;
    }

    fn set_frame_performance_stats(&self, stats: dyxel_perf::FramePerformanceStats) {
        *self.frame_perf_stats.lock().unwrap() = stats;
    }

    fn render_package(
        &self,
        device: DeviceHandle,
        queue: QueueHandle,
        surface: &mut dyn SurfaceState,
        package: &dyxel_render_api::RenderPackage,
    ) -> RenderResult {
        let device = unsafe { &*device.as_ptr::<wgpu::Device>() };
        let queue = unsafe { &*queue.as_ptr::<wgpu::Queue>() };

        #[cfg(target_os = "macos")]
        {
            let v_surface = surface
                .as_any_mut()
                .downcast_mut::<mac::MacVelloSurfaceState>()
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid surface state (not MacVelloSurfaceState)")
                })?;
            let st = v_surface
                .surface
                .surface
                .get_current_texture()
                .map_err(|e| anyhow::anyhow!("Failed to get current texture: {:?}", e))?;
            let target_view = st.texture.create_view(&Default::default());
            let _ = self.render_internal_impl(
                device,
                queue,
                &target_view,
                v_surface.surface.format,
                package,
            )?;
            st.present();
            return Ok(());
        }

        #[cfg(target_os = "android")]
        {
            let v_surface = surface
                .as_any_mut()
                .downcast_mut::<android::AndroidVelloSurfaceState>()
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid surface state (not AndroidVelloSurfaceState)")
                })?;
            let st = v_surface
                .surface
                .surface
                .get_current_texture()
                .map_err(|e| anyhow::anyhow!("Failed to get current texture: {:?}", e))?;
            let target_view = st.texture.create_view(&Default::default());
            let _ = self.render_internal_impl(
                device,
                queue,
                &target_view,
                v_surface.surface.format,
                package,
            )?;
            st.present();
            return Ok(());
        }

        #[cfg(target_arch = "wasm32")]
        {
            let v_surface = surface
                .as_any_mut()
                .downcast_mut::<web::WebVelloSurfaceState>()
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid surface state (not WebVelloSurfaceState)")
                })?;
            let st = v_surface
                .surface
                .surface
                .get_current_texture()
                .map_err(|e| anyhow::anyhow!("Failed to get current texture: {:?}", e))?;
            let target_view = st.texture.create_view(&Default::default());
            let _ = self.render_internal_impl(
                device,
                queue,
                &target_view,
                v_surface.surface.format,
                package,
            )?;
            st.present();
            return Ok(());
        }

        #[cfg(all(
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_arch = "wasm32")
        ))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn on_lifecycle_event(&self, event: LifecycleEvent) {
        match event {
            LifecycleEvent::FirstFrameDone | LifecycleEvent::Shutdown => {
                self.save_cache();
            }
            _ => {}
        }
    }

    fn sync_gpu(&self, _device: DeviceHandle, queue: QueueHandle) {
        let queue = unsafe { &*queue.as_ptr::<wgpu::Queue>() };

        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        queue.on_submitted_work_done(move || {
            let _ = tx.send(());
        });

        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(_) => log::info!("VelloBackend: sync_gpu completed successfully"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                log::warn!("VelloBackend: sync_gpu timed out, GPU may be unresponsive");
            }
            Err(e) => log::error!("VelloBackend: sync_gpu error: {:?}", e),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl RenderBackendExt for VelloBackend {
    fn enable_perf_overlay(&self) {
        self.enable_perf_overlay();
    }

    fn disable_perf_overlay(&self) {
        self.disable_perf_overlay();
    }
}

impl VelloBackendExt for VelloBackend {
    fn vello_renderer(&self) -> Option<&dyn Any> {
        // Return the backend itself as Any, caller can downcast to VelloBackend
        // and access renderer through the public renderer field
        Some(self as &dyn Any)
    }
}

impl Default for VelloBackend {
    fn default() -> Self {
        Self::new()
    }
}

// Re-export new double-layer types at crate root for convenience.
pub use backend::VelloDrawingBackend;
pub use factory::{VelloBackendFactory, VelloGraphicsFactory};
pub use frame_context::WgpuFrameContext;
pub use runtime::WgpuRuntime;

#[cfg(test)]
mod tests {
    use super::*;

    /// Test coordinate transformation for blur sampling
    /// Verifies that source rectangle is calculated correctly for two-pass blur
    #[test]
    fn test_blur_source_rect_calculation() {
        // Simulate a view at position (100, 200) with size 120x120
        let local_transform = Affine::translate((100.0, 200.0));
        let node_width = 120.0;
        let node_height = 120.0;

        // Extract translation (same logic as render_with_blur)
        let mat = local_transform.as_coeffs();
        let source_x = mat[4] as f32; // translation x
        let source_y = mat[5] as f32; // translation y

        assert_eq!(source_x, 100.0);
        assert_eq!(source_y, 200.0);

        // Verify source_rect tuple
        let source_rect = (source_x, source_y, node_width as f32, node_height as f32);
        assert_eq!(source_rect, (100.0, 200.0, 120.0, 120.0));
    }

    /// Test Y-coordinate handling for blur source copies.
    /// Pass 1 already renders/blits the scene texture in screen Y-down order,
    /// so Pass 2 must not apply an extra Android flip.
    #[test]
    fn test_blur_copy_y_uses_screen_y() {
        let src_y = 200.0f32;

        let copy_y = src_y.max(0.0) as u32;
        assert_eq!(copy_y, 200);
    }

    /// Test padding calculation for blur bleed
    #[test]
    fn test_blur_padding_calculation() {
        let blur_radius = 10.0f64;
        let padding = (blur_radius * 2.5).ceil() as u32;

        // Expected: 10.0 * 2.5 = 25.0, ceil = 25
        assert_eq!(padding, 25);

        let texture_width = (120.0f64 as u32 + padding * 2).max(1);
        let texture_height = (120.0f64 as u32 + padding * 2).max(1);

        // Expected: 120 + 25*2 = 170
        assert_eq!(texture_width, 170);
        assert_eq!(texture_height, 170);
    }

    /// Test transform adjustment for padding offset
    #[test]
    fn test_transform_with_padding_offset() {
        let local_transform = Affine::translate((100.0, 200.0));
        let blur_radius = 10.0f64;
        let padding = (blur_radius * 2.5).ceil() as u32;

        // Adjust transform to account for padding offset
        let final_transform =
            local_transform * Affine::translate((-(padding as f64), -(padding as f64)));
        let final_mat = final_transform.as_coeffs();

        // Translation should be offset by padding
        assert_eq!(final_mat[4], 100.0 - padding as f64); // x: 100 - 25 = 75
        assert_eq!(final_mat[5], 200.0 - padding as f64); // y: 200 - 25 = 175
    }

    /// Test that padding is consistent between texture creation and transform
    #[test]
    fn test_padding_consistency() {
        let blur_radius = 10.0;
        let node_width = 120.0;
        let node_height = 120.0;

        // Calculate padding (same as render_with_blur)
        let padding = (blur_radius as f64 * 2.5).ceil() as u32;

        // Texture size with padding
        let texture_width = (node_width as u32 + padding * 2).max(1);
        let texture_height = (node_height as u32 + padding * 2).max(1);

        // Verify padding is applied equally on both sides
        let inner_width = texture_width - padding * 2;
        let inner_height = texture_height - padding * 2;

        assert_eq!(inner_width, node_width as u32);
        assert_eq!(inner_height, node_height as u32);
    }

    /// Test frosted glass color extraction
    #[test]
    fn test_frosted_glass_color() {
        // Color from layer_effects_demo.rs: (255u32, 255, 255, 180)
        let color = (255u32, 255, 255, 180);

        // Convert to f32 premultiplied (as done in blur_composite.wgsl)
        let alpha = color.3 as f32 / 255.0;
        let r = (color.0 as f32 / 255.0) * alpha;
        let g = (color.1 as f32 / 255.0) * alpha;
        let b = (color.2 as f32 / 255.0) * alpha;

        // White with 180/255 alpha should have premultiplied values
        assert!((r - 0.705).abs() < 0.01, "R should be ~0.705, got {}", r);
        assert!((g - 0.705).abs() < 0.01, "G should be ~0.705, got {}", g);
        assert!((b - 0.705).abs() < 0.01, "B should be ~0.705, got {}", b);
    }
}
