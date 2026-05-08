// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_perf::PerfConfig;
use dyxel_render_api::{RenderBackendExt, VelloBackendExt};
use std::any::Any;
use vello::wgpu;
use vello::{Renderer, Scene};

mod blur;
mod cache;
mod cold_start;
pub(crate) mod color;
mod coordinates;
mod debug_utils;
#[cfg(test)]
mod experimental;
mod frame;
mod legacy_backend;
mod render_helpers;
mod scene_renderer;
mod shadow;
mod state;
pub(crate) mod text;
use cache::CachedDraw;
pub use coordinates::platform_correction;

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

/// Vello renderer state and pass coordinator.
///
/// `backend::VelloDrawingBackend` uses this for the active double-layer
/// `RenderBackendV2` path; `legacy_backend` wires it to the older direct
/// `RenderBackend` compatibility API.
pub struct VelloBackend {
    renderer_state: state::RendererState,
    blit_state: state::BlitState,
    blur_state: state::BlurState,
    raster_cache_state: state::RasterCacheState,
    shadow_cache_state: state::ShadowCacheState,
    text_cache_state: state::TextCacheState,
    diagnostics_state: state::DiagnosticsState,
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
            renderer_state: state::RendererState::new(memory_optimizer),
            blit_state: state::BlitState::new(),
            blur_state: state::BlurState::new(),
            raster_cache_state: state::RasterCacheState::new(),
            shadow_cache_state: state::ShadowCacheState::new(),
            text_cache_state: state::TextCacheState::new(),
            diagnostics_state: state::DiagnosticsState::new(perf_config),
        }
    }

    /// Access the lazily initialized Vello renderer for diagnostics/tests.
    pub fn renderer_handle(&self) -> &std::sync::Arc<std::sync::Mutex<Option<Renderer>>> {
        self.renderer_state.renderer_handle()
    }

    /// Enable performance overlay
    pub fn enable_perf_overlay(&self) {
        self.diagnostics_state.toggle_perf_overlay();
    }

    /// Disable performance overlay
    pub fn disable_perf_overlay(&self) {
        self.diagnostics_state.disable_perf_overlay();
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
        self.blur_state
            .collect_returned_textures_and_reset_staging();

        // Detailed frame timing for diagnostics
        let frame_start = std::time::Instant::now();
        let mut stage_timer = dyxel_perf::FrameTimer::new();

        // Async initialization: start background compilation without blocking
        self.ensure_renderer_initialized_async(device, queue);
        stage_timer.mark("init_check");

        // Check if renderer is ready
        let mut renderer_lock = self.renderer_state.renderer_lock();
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
        self.diagnostics_state.begin_frame();
        stage_timer.mark("perf_start");

        if w == 0 || h == 0 {
            return Ok(None);
        }

        // Reset per-frame shadow cache miss counter
        self.shadow_cache_state.reset_frame_budget();

        // Shadow cache LRU eviction: remove entries unused for 300 frames (5s @ 60fps)
        self.shadow_cache_state.evict_stale_entries(
            renderer,
            FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed),
        );

        // Detect renderer replacement (e.g. Stage 1 -> Stage 2 cold-start upgrade).
        // When renderer is replaced, image_overrides are lost; stale cache entries
        // would trigger "invalid empty image" panic on draw_image.
        self.shadow_cache_state
            .sync_renderer_id(renderer, self.renderer_state.current_renderer_id());

        // Glyph run cache LRU eviction: remove entries unused for 300 frames
        self.text_cache_state
            .evict_stale_glyph_runs(FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed));

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
        self.raster_cache_state.execute_bake_plans(
            package,
            device,
            queue,
            renderer,
            |node_id, bake_scene, cached_lookup, renderer| {
                self.build_raster_cache_scene(
                    node_id,
                    &node_map,
                    bake_scene,
                    device,
                    queue,
                    renderer,
                    cached_lookup,
                );
            },
        );
        stage_timer.mark("bake_done");

        self.blit_state.ensure_triple_buffer(device, queue, w, h);
        let tb = self
            .blit_state
            .current_triple_buffer_slot()
            .expect("triple buffer should be initialized");
        let aa_config = self.renderer_state.current_aa_config();
        self.renderer_state.render_scene_to_texture(
            device,
            queue,
            renderer,
            &scene,
            &tb.view,
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
        self.debug_save_pass1_scene_texture(device, queue, &tb.texture);

        // === PASS 2: Process blur textures from scene ===
        let mut post_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Frame Encoder"),
        });
        let (atlas_wide_blur_valid_this_frame, atlas_wide_source_copies_this_frame) = self
            .blur_state
            .process_blur_pass2(device, &mut post_enc, &tb.texture, w, h, &mut stage_timer);
        let has_blur = self.blur_state.has_blur_entries();

        stage_timer.mark("pass3_start");

        // === PASS 3: Render deferred children to per-entry local textures ===
        // Only run when there are actual blur entries with deferred children.
        if has_blur {
            #[allow(unused_variables)]
            let rendered_indices = self.blur_state.render_pass3_children(
                &node_map,
                device,
                queue,
                renderer,
                aa_config,
                &self.text_cache_state,
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
                self.blur_state.for_each_rendered_child_texture(
                    &rendered_indices,
                    |view_id, tex| {
                        let path = debug_dir.join(format!(
                            "frame_{:06}_pass3_children_view_{}.png",
                            frame_num % 1000,
                            view_id
                        ));
                        self.save_texture_to_png(device, queue, tex, path.to_str().unwrap());
                    },
                );
            }
        }
        stage_timer.mark("pass3_done");

        // === PASS 3.5: Pack blur textures into atlas for instanced composite ===
        let atlas_sampler = self.blit_state.sampler_clone();
        let (atlas_bind_group, atlas_instance_count, atlas_enabled_this_frame) =
            self.blur_state.pack_blur_atlas_pass(
                device,
                queue,
                &mut post_enc,
                atlas_sampler.as_ref(),
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
            self.blit_state.draw_bind_group(&mut rp, &tb.bind_group);

            let sampler = self
                .blit_state
                .sampler_clone()
                .expect("Sampler should be initialized");
            _had_blur_textures =
                self.raster_cache_state
                    .with_gpu_texture_pool(|gpu_texture_pool| {
                        self.blur_state.composite_blur_pass(
                            &mut rp,
                            has_blur,
                            &cached_draws,
                            device,
                            queue,
                            w,
                            h,
                            surface_format,
                            &sampler,
                            gpu_texture_pool,
                            atlas_bind_group.as_ref(),
                            atlas_instance_count,
                            atlas_enabled_this_frame,
                        )
                    });
        }

        // If using capture texture, blit it to surface before present (same encoder)
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(ref capture_tex) = capture_texture {
            self.ensure_blit_pipeline(device, surface_format);
            let capture_view = capture_tex.create_view(&Default::default());
            let capture_bind_group = self.blit_state.create_texture_bind_group(
                device,
                "Capture Blit Bind Group",
                &capture_view,
            );

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
                self.blit_state
                    .draw_bind_group(&mut rp, &capture_bind_group);
            }
        }

        // Single submit for all post-Vello GPU work
        let submission_index = queue.submit(Some(post_enc.finish()));
        stage_timer.mark("blit_submit");

        // Debug: Save composite frame when we have blur textures
        #[cfg(not(target_arch = "wasm32"))]
        {
            log::debug!(
                "[Debug] Checking had_blur_textures = {}",
                _had_blur_textures
            );
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
        self.diagnostics_state.log_frame_diagnostics(
            &self.shadow_cache_state,
            &self.text_cache_state,
            &stage_timer,
            frame_start,
        );

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
    use kurbo::Affine;

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
