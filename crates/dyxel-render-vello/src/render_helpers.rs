// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Render helper methods: frame setup, scene building, cache management, and diagnostics.

use crate::blur::{
    apply_legacy_kawase_blur, collect_blur_dirty_report, compute_blur_atlas_layout,
    copy_legacy_blur_sources, log_blur_dirty_report, pack_blur_atlas,
    select_legacy_rebuild_indices, BlurDirtyKind, BlurredTextureEntry, BLUR_ATLAS_LEGACY_GAP_PX,
    BLUR_ATLAS_WIDE_GAP_PX, USE_ATLAS_WIDE_BACKDROP_BLUR, USE_FULL_FRAME_BACKDROP_BLUR,
};
use crate::cache::CachedDraw;
use crate::coordinates::platform_correction;
use crate::frame::{TripleBuffer, TripleBufferSlot};
use crate::{VelloBackend, BLUR_SCENE_FRAME, DIAG_LOG_EVERY_N_FRAMES, FRAME_COUNTER, NODE_COUNTER};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use kurbo::{Affine, Vec2};
use vello::peniko::Color;
use vello::{Renderer, Scene};

impl VelloBackend {
    #[inline]
    pub(crate) fn clear_surface(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_view: &wgpu::TextureView,
    ) -> anyhow::Result<Option<vello::wgpu::SubmissionIndex>> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Clear Surface (Async Loading)"),
        });

        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), // Clear to black
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        let submission_index = queue.submit(Some(encoder.finish()));
        // Present is handled by the caller (old render_package) or GraphicsRuntime::end_frame (new path)

        Ok(Some(submission_index))
    }

    #[inline]
    pub(crate) fn collect_returned_textures_and_reset_blur_staging(&self) {
        if let Some(ref pool) = *self.texture_pool.lock().unwrap() {
            pool.collect_returns();
        }
        self.blur_staging_offset
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn reset_shadow_frame_budget(&self) {
        self.shadow_cache_misses_this_frame
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn evict_stale_shadow_cache_entries(&self, renderer: &mut Renderer) {
        let current_frame = FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
        let mut cache = self.shadow_cache.lock().unwrap();
        let mut stats = self.shadow_cache_stats.lock().unwrap();
        let before = cache.len();
        let evicted: Vec<peniko::ImageData> = cache
            .extract_if(|_, entry| {
                let last = entry
                    .last_used_frame
                    .load(std::sync::atomic::Ordering::Relaxed);
                current_frame.saturating_sub(last) > 300
            })
            .map(|(_, entry)| entry.image_data)
            .collect();
        let after = cache.len();
        if before != after {
            stats.evictions += (before - after) as u64;
            log::debug!(
                "[ShadowCache] Evicted {} entries ({} -> {})",
                before - after,
                before,
                after
            );
        }
        // Unregister evicted textures from renderer to prevent image_overrides bloat.
        drop(cache);
        drop(stats);
        for image_data in evicted {
            renderer.unregister_texture(image_data);
        }
    }

    #[inline]
    pub(crate) fn sync_shadow_cache_renderer_id(&self, renderer: &mut Renderer) {
        let current_id = self.renderer_id.load(std::sync::atomic::Ordering::Relaxed);
        let last_id = self
            .shadow_cache_renderer_id
            .load(std::sync::atomic::Ordering::Relaxed);
        if last_id != 0 && last_id != current_id {
            log::warn!(
                "[ShadowCache] Renderer replaced (id {} -> {}), clearing shadow cache",
                last_id,
                current_id
            );
            let mut cache = self.shadow_cache.lock().unwrap();
            for (_, entry) in cache.drain() {
                renderer.unregister_texture(entry.image_data);
            }
        }
        self.shadow_cache_renderer_id
            .store(current_id, std::sync::atomic::Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn evict_stale_glyph_runs(&self) {
        let current_frame = FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
        let mut cache = self.glyph_run_cache.lock().unwrap();
        let mut stats = self.glyph_run_cache_stats.lock().unwrap();
        let before = cache.len();
        cache.retain(|_, entry| {
            let last = entry
                .last_used_frame
                .load(std::sync::atomic::Ordering::Relaxed);
            let keep = current_frame.saturating_sub(last) <= 300;
            if !keep {
                stats.evictions += 1;
            }
            keep
        });
        let after = cache.len();
        if before != after {
            log::debug!(
                "[GlyphCache] Evicted {} entries ({} -> {})",
                before - after,
                before,
                after
            );
        }
    }

    #[inline]
    pub(crate) fn build_main_scene(
        &self,
        root_id: Option<u32>,
        node_map: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
        scene: &mut Scene,
        cached_draws: &mut Vec<CachedDraw>,
        viewport_h: u32,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
    ) {
        if let Some(id) = root_id {
            // DEBUG: Reset node counter for this frame.
            NODE_COUNTER.store(0, std::sync::atomic::Ordering::SeqCst);

            // Apply platform correction at the root level.
            let root_transform = platform_correction(viewport_h as f64);
            let blur_scene_frame =
                BLUR_SCENE_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;

            // Get filter pipeline for blur effects.
            let filter_pipeline = self.filter_pipeline.lock().unwrap();
            let mut blurred_textures = self.blurred_textures.lock().unwrap();
            // Keep entries across frames so blur/children textures can be reused.
            // Stale entries are removed after the scene walk via last_seen_frame.

            self.render_node_recursive_with_transform(
                id,
                node_map,
                scene,
                Vec2::ZERO,
                root_transform,
                device,
                queue,
                renderer,
                filter_pipeline.as_ref(),
                &mut blurred_textures,
                cached_draws,
                false,
                blur_scene_frame,
            );
            blurred_textures.retain(|entry| entry.last_seen_frame == blur_scene_frame);
        }
    }

    #[inline]
    pub(crate) fn execute_raster_cache_plans(
        &self,
        package: &dyxel_render_api::RenderPackage,
        node_map: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
    ) {
        // === Execute recycle plans produced by Runtime cache policy ===
        {
            let mut cached_textures_guard = self.cached_textures.lock().unwrap();
            let mut gpu_texture_pool = self.gpu_texture_pool.lock().unwrap();
            if let Some(pool) = gpu_texture_pool.as_mut() {
                for plan in &package.recycle_plans {
                    pool.release(crate::texture_pool::TextureId(plan.texture_id.0));
                    cached_textures_guard.remove(&plan.node_id);
                }
            }
        }

        // === Execute bake plans produced by Runtime cache policy ===
        // LIMIT: process at most 2 bake plans per frame to prevent render time spikes.
        const MAX_BAKES_PER_FRAME: usize = 2;
        let mut cached_textures_guard = self.cached_textures.lock().unwrap();
        let mut gpu_texture_pool = self.gpu_texture_pool.lock().unwrap();
        if let Some(pool) = gpu_texture_pool.as_mut() {
            for plan in package.bake_plans.iter().take(MAX_BAKES_PER_FRAME) {
                let tex_w = plan.width;
                let tex_h = plan.height;
                if tex_w == 0 || tex_h == 0 {
                    continue;
                }

                let texture_id = pool.acquire(tex_w, tex_h, wgpu::TextureFormat::Rgba8Unorm);
                if let Some(ptex) = pool.get_texture(texture_id) {
                    let mut bake_scene = Scene::new();
                    let mut bake_blurred = Vec::new();
                    self.render_node_recursive_internal(
                        plan.node_id,
                        node_map,
                        &mut bake_scene,
                        Vec2::ZERO,
                        Affine::IDENTITY,
                        device,
                        queue,
                        renderer,
                        None,
                        &mut bake_blurred,
                        &*cached_textures_guard,
                        &mut Vec::new(),
                        false,
                        0,
                    );
                    let _ = renderer.render_to_texture(
                        device,
                        queue,
                        &bake_scene,
                        ptex.view(),
                        &vello::RenderParams {
                            base_color: Color::TRANSPARENT,
                            width: tex_w,
                            height: tex_h,
                            antialiasing_method: vello::AaConfig::Area,
                        },
                    );
                    cached_textures_guard.insert(
                        plan.node_id,
                        dyxel_render_api::raster_cache::TextureId(texture_id.0),
                    );
                }
            }
        }
    }

    #[inline]
    pub(crate) fn ensure_triple_buffer(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
    ) {
        // Triple-buffering: create / resize the ring when dimensions change.
        let mut triple_buffer = self.triple_buffer.lock().unwrap();
        let needs_recreate = triple_buffer
            .as_ref()
            .map_or(true, |tb| tb.width != w || tb.height != h);
        if needs_recreate {
            let layout = self
                .blit_bind_group_layout
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .clone();
            let sampler = self.sampler.lock().unwrap().as_ref().unwrap().clone();

            let make_slot = || {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Vello Offscreen Texture (TripleBuffer)"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
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
                let view = texture.create_view(&Default::default());
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Vello Blit Bind Group (TripleBuffer)"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });
                TripleBufferSlot {
                    texture,
                    view,
                    bind_group,
                }
            };

            let tb_new = TripleBuffer {
                slots: [make_slot(), make_slot(), make_slot()],
                current_index: 0,
                width: w,
                height: h,
            };

            // Cold-start fix: initialize newly-created GPU textures to transparent.
            // Without this, uninitialized texture memory may display as white/gray
            // during the first frame while shaders are still compiling.
            let mut init_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("TripleBuffer Init Clear"),
            });
            for slot in &tb_new.slots {
                init_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Init Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &slot.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            queue.submit([init_enc.finish()]);

            *triple_buffer = Some(tb_new);
        }

        triple_buffer.as_mut().unwrap().advance();
    }

    #[inline]
    pub(crate) fn render_main_scene_to_texture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
        scene: &Scene,
        target_view: &wgpu::TextureView,
        w: u32,
        h: u32,
        aa_config: vello::AaConfig,
        diag_log_this_frame: bool,
    ) -> anyhow::Result<()> {
        // Single render: main scene + overlay (if enabled) to offscreen texture.
        log::debug!("[Blur] Rendering scene to texture {}x{}", w, h);
        let enc = scene.encoding();
        if diag_log_this_frame {
            log::info!(
                "[DIAG] Scene encoding: empty={} n_paths={} n_clips={} n_open_clips={} path_tags={} draw_tags={}",
                enc.is_empty(),
                enc.n_paths,
                enc.n_clips,
                enc.n_open_clips,
                enc.path_tags.len(),
                enc.draw_tags.len()
            );
        }

        renderer
            .render_to_texture(
                device,
                queue,
                scene,
                target_view,
                &vello::RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: w,
                    height: h,
                    antialiasing_method: aa_config,
                },
            )
            .map_err(|e| anyhow::anyhow!("render_to_texture failed: {:?}", e))
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub(crate) fn debug_save_pass1_scene_texture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_texture: &wgpu::Texture,
    ) {
        if !self.debug_frames_enabled() {
            return;
        }

        let frame_num = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let debug_dir = self.debug_output_dir();
        let path = debug_dir.join(format!("frame_{:06}_pass1_scene.png", frame_num % 1000));
        self.save_texture_to_png(device, queue, scene_texture, path.to_str().unwrap());

        // Debug: Sample pixels at blur card locations (expected to show purple background).
        log::debug!("[Debug] Sampling scene texture at blur card locations (expected purple bg)");
    }

    #[inline]
    pub(crate) fn current_aa_config(&self) -> vello::AaConfig {
        // Tier-based AA configuration: reduce quality for LowEnd to save memory.
        let multiplier = self
            .memory_optimizer
            .lock()
            .unwrap()
            .vello_buffer_multiplier();
        if multiplier < 0.5 {
            vello::AaConfig::Area // LowEnd: use simpler AA
        } else {
            vello::AaConfig::Area // Default to Area for consistent performance
        }
    }

    #[inline]
    pub(crate) fn run_full_frame_backdrop_blur_branch(
        &self,
        device: &wgpu::Device,
        post_enc: &mut wgpu::CommandEncoder,
        pipeline: Option<&crate::filter_pipeline::FilterPipeline>,
        scene_texture: &wgpu::Texture,
        blurred_textures: &mut [BlurredTextureEntry],
        w: u32,
        h: u32,
        max_radius: f32,
        stage_timer: &mut dyxel_perf::FrameTimer,
    ) {
        if let Some(pipeline) = pipeline {
            self.ensure_backdrop_blur_texture(device, w, h);
            let backdrop_texture = {
                let backdrop = self.backdrop_blur.lock().unwrap();
                backdrop.as_ref().map(|b| b.texture.clone())
            };
            if let Some(backdrop_texture) = backdrop_texture {
                post_enc.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: scene_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &backdrop_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                );
                stage_timer.mark("blur_copy_submit");
                let pool_guard = self.texture_pool.lock().unwrap();
                if let Err(e) = pipeline.apply_frosted_glass_kawase(
                    post_enc,
                    &backdrop_texture,
                    &backdrop_texture,
                    max_radius,
                    pool_guard.as_ref(),
                ) {
                    log::warn!("[BlurBackdropFull] Kawase failed: {:?}", e);
                }
                for entry in blurred_textures.iter_mut() {
                    entry.blur_valid = true;
                    entry.blur_rebuild_pending = false;
                    entry.dirty_kind = BlurDirtyKind::Clean;
                }
                stage_timer.mark("blur_render_submit");
                return;
            }
        }

        stage_timer.mark("blur_copy_submit");
        stage_timer.mark("blur_render_submit");
    }

    /// Pass 2: Process blur textures — atlas-wide blur, legacy per-entry blur, or skip.
    /// Returns `(atlas_wide_blur_valid, atlas_wide_source_copies)`.
    #[inline]
    pub(crate) fn process_blur_pass2(
        &self,
        device: &wgpu::Device,
        post_enc: &mut wgpu::CommandEncoder,
        scene_texture: &wgpu::Texture,
        w: u32,
        h: u32,
        stage_timer: &mut dyxel_perf::FrameTimer,
    ) -> (bool, usize) {
        let has_blur = !self.blurred_textures.lock().unwrap().is_empty();

        if !USE_FULL_FRAME_BACKDROP_BLUR {
            *self.backdrop_blur.lock().unwrap() = None;
        }

        let mut atlas_wide_blur_valid = false;
        let mut atlas_wide_source_copies = 0usize;

        if has_blur {
            let current_frame = FRAME_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut blurred_textures = self.blurred_textures.lock().unwrap();
            let filter_pipeline = self.filter_pipeline.lock().unwrap();
            let dirty_report = collect_blur_dirty_report(&blurred_textures, w, h);
            let max_radius = dirty_report.max_radius;
            if current_frame % DIAG_LOG_EVERY_N_FRAMES == 0 {
                log_blur_dirty_report(current_frame, blurred_textures.len(), &dirty_report);
            }

            if USE_FULL_FRAME_BACKDROP_BLUR {
                self.run_full_frame_backdrop_blur_branch(
                    device,
                    post_enc,
                    filter_pipeline.as_ref(),
                    scene_texture,
                    &mut blurred_textures,
                    w,
                    h,
                    max_radius,
                    stage_timer,
                );
            } else if let Some(pipeline) = filter_pipeline.as_ref() {
                if USE_ATLAS_WIDE_BACKDROP_BLUR {
                    let (valid, copies) = self.try_atlas_wide_blur(
                        device,
                        post_enc,
                        pipeline,
                        scene_texture,
                        &mut blurred_textures,
                        w,
                        h,
                        max_radius,
                        current_frame,
                        stage_timer,
                    );
                    atlas_wide_blur_valid = valid;
                    atlas_wide_source_copies = copies;
                }

                if !atlas_wide_blur_valid {
                    if self
                        .blur_atlas_wide_active_last_frame
                        .swap(false, std::sync::atomic::Ordering::Relaxed)
                    {
                        for entry in blurred_textures.iter_mut() {
                            entry.blur_valid = false;
                            entry.blur_rebuild_pending = true;
                            entry.atlas_valid = false;
                            entry.atlas_dirty = true;
                        }
                    }
                    let rebuild_indices = select_legacy_rebuild_indices(&blurred_textures, w, h);
                    if !rebuild_indices.is_empty() && current_frame % DIAG_LOG_EVERY_N_FRAMES == 0 {
                        log::info!(
                            "[BlurLegacy] Budget: rebuilding {} pending entries",
                            rebuild_indices.len()
                        );
                    }
                    let blur_entries = copy_legacy_blur_sources(
                        &mut blurred_textures,
                        post_enc,
                        scene_texture,
                        &rebuild_indices,
                        w,
                        h,
                    );
                    stage_timer.mark("blur_copy_submit");
                    {
                        let pool_guard = self.texture_pool.lock().unwrap();
                        apply_legacy_kawase_blur(
                            &mut blurred_textures,
                            pipeline,
                            post_enc,
                            blur_entries,
                            pool_guard.as_ref(),
                            current_frame,
                        );
                    }
                    stage_timer.mark("blur_render_submit");
                } else {
                    self.blur_atlas_wide_active_last_frame
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            } else {
                self.blur_atlas_wide_active_last_frame
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                stage_timer.mark("blur_copy_submit");
                stage_timer.mark("blur_render_submit");
            }
        } else {
            self.blur_atlas_wide_active_last_frame
                .store(false, std::sync::atomic::Ordering::Relaxed);
            stage_timer.mark("blur_copy_submit");
            stage_timer.mark("blur_render_submit");
        }

        (atlas_wide_blur_valid, atlas_wide_source_copies)
    }

    /// Pass 3.5: Pack valid blur textures into an atlas for instanced composite.
    /// Returns `(atlas_bind_group, atlas_instance_count, atlas_enabled)`.
    #[inline]
    pub(crate) fn pack_blur_atlas_pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        post_enc: &mut wgpu::CommandEncoder,
        surface_format: wgpu::TextureFormat,
        atlas_wide_blur_valid: bool,
        atlas_wide_source_copies: usize,
        w: u32,
        h: u32,
    ) -> (Option<wgpu::BindGroup>, u32, bool) {
        let mut atlas_bind_group: Option<wgpu::BindGroup> = None;
        let mut atlas_instance_count: u32 = 0;
        let mut atlas_enabled = false;

        let has_blur = !self.blurred_textures.lock().unwrap().is_empty();
        if !has_blur {
            return (atlas_bind_group, atlas_instance_count, atlas_enabled);
        }

        let mut blurred_textures = self.blurred_textures.lock().unwrap();
        let gap = if atlas_wide_blur_valid {
            BLUR_ATLAS_WIDE_GAP_PX
        } else {
            BLUR_ATLAS_LEGACY_GAP_PX
        };
        if let Some(layout) = compute_blur_atlas_layout(&blurred_textures, w, h, gap) {
            if layout.placements.len() >= 8 {
                self.ensure_blur_instanced_resources(
                    device,
                    surface_format,
                    layout.placements.len(),
                );
                let atlas_recreated =
                    self.ensure_blur_atlas_texture(device, layout.width, layout.height);
                if atlas_recreated && !atlas_wide_blur_valid {
                    for entry in blurred_textures.iter_mut() {
                        entry.atlas_valid = false;
                        entry.atlas_dirty = true;
                    }
                }

                let atlas_guard = self.blur_atlas.lock().unwrap();
                let frame_buf_guard = self.blur_frame_uniform.lock().unwrap();
                let inst_buf_guard = self.blur_instance_buffer.lock().unwrap();
                let bg_layout_guard = self.blur_instanced_bind_group_layout.lock().unwrap();
                let mut cached_bg_guard = self.blur_instanced_bind_group.lock().unwrap();
                let sampler_guard = self.sampler.lock().unwrap();

                if let (
                    Some(atlas),
                    Some(frame_buf),
                    Some(inst_buf),
                    Some(bg_layout),
                    Some(sampler),
                ) = (
                    atlas_guard.as_ref(),
                    frame_buf_guard.as_ref(),
                    inst_buf_guard.as_ref(),
                    bg_layout_guard.as_ref(),
                    sampler_guard.as_ref(),
                ) {
                    let result = pack_blur_atlas(
                        &mut blurred_textures,
                        device,
                        queue,
                        post_enc,
                        atlas,
                        atlas_wide_blur_valid,
                        &layout,
                        frame_buf,
                        inst_buf,
                        bg_layout,
                        &mut cached_bg_guard,
                        sampler,
                        w,
                        h,
                        atlas_wide_source_copies,
                        DIAG_LOG_EVERY_N_FRAMES,
                        FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed),
                    );
                    atlas_bind_group = result.bind_group;
                    atlas_instance_count = result.instance_count;
                    atlas_enabled = result.enabled;
                }

                if atlas_wide_blur_valid && !atlas_enabled {
                    for entry in blurred_textures.iter_mut() {
                        entry.blur_valid = false;
                        entry.blur_rebuild_pending = true;
                        entry.atlas_valid = false;
                        entry.atlas_dirty = true;
                    }
                }
            }
        }

        (atlas_bind_group, atlas_instance_count, atlas_enabled)
    }

    #[inline]
    pub(crate) fn log_frame_diagnostics(
        &self,
        stage_timer: &dyxel_perf::FrameTimer,
        frame_start: std::time::Instant,
    ) {
        let _pacer_wait_ms = *self.pacer_wait_ms.lock().unwrap();
        let frame_interval_ms = *self.frame_interval_ms.lock().unwrap();
        let perf_stats = self.frame_perf_stats.lock().unwrap();
        let stats = self.perf_monitor.lock().unwrap().get_stats();
        {
            let report = stage_timer.report();

            let state_lock_time =
                report.get("init_check_to_perf_start") + report.get("perf_start_to_state_lock");
            let scene_build_time = report.get("state_lock_to_scene_build");
            let bake_time = report.get("scene_build_to_bake_done");
            let gpu_time = report.get("bake_done_to_gpu_render");
            let blur_copy_time = report.get("gpu_render_to_blur_copy_submit");
            let blur_render_time = report.get("blur_copy_submit_to_blur_render_submit");
            let pass3_time = report.get("blur_render_submit_to_pass3_done");
            // Surface texture is acquired in GraphicsRuntime::begin_frame, not inside backend.
            let get_texture_time = 0.0;
            let texture_wait_time = 0.0;
            let blit_time = report.get("surface_ready_to_blit_submit");
            let submit_return_time = report.get("blit_submit_to_render_return");
            let total = frame_start.elapsed().as_secs_f32() * 1000.0;

            if stats.total_frames % DIAG_LOG_EVERY_N_FRAMES == 0 || total > 18.0 {
                log::info!(
                    "[DIAG-BACKEND] Frame {}: Total={:.2}ms, UI={:.1}fps, Raster={:.1}fps, Target={:.1}fps, Jank={}({:.1}%), Drop={}({:.1}%) | State={:.2}ms, Scene={:.2}ms, Bake={:.2}ms, GPU={:.2}ms, BlurCopy={:.2}ms, BlurRender={:.2}ms, Pass3={:.2}ms, GetTex={:.2}ms, TexWait={:.2}ms, Blit={:.2}ms, SubmitReturn={:.2}ms, Interval={:.2}ms",
                    stats.total_frames,
                    total,
                    perf_stats.ui_fps,
                    perf_stats.raster_fps,
                    perf_stats.target_fps,
                    perf_stats.jank_count,
                    perf_stats.jank_rate * 100.0,
                    perf_stats.dropped_count,
                    perf_stats.drop_rate * 100.0,
                    state_lock_time,
                    scene_build_time,
                    bake_time,
                    gpu_time,
                    blur_copy_time,
                    blur_render_time,
                    pass3_time,
                    get_texture_time,
                    texture_wait_time,
                    blit_time,
                    submit_return_time,
                    frame_interval_ms,
                );
            }

            // Shadow cache DIAG logging
            if stats.total_frames % 60 == 0 {
                let cache_stats = self.shadow_cache_stats.lock().unwrap();
                let cache_size = self.shadow_cache.lock().unwrap().len();
                let total = cache_stats.hits + cache_stats.misses;
                if total > 0 {
                    log::info!(
                        "[DIAG] ShadowCache: size={} hits={} misses={} hit_rate={:.1}% evictions={}",
                        cache_size,
                        cache_stats.hits,
                        cache_stats.misses,
                        (cache_stats.hits as f64 / total as f64) * 100.0,
                        cache_stats.evictions
                    );
                }
            }

            // Glyph run cache DIAG logging
            if stats.total_frames % 60 == 0 {
                let cache_stats = self.glyph_run_cache_stats.lock().unwrap();
                let cache_size = self.glyph_run_cache.lock().unwrap().len();
                let total = cache_stats.hits + cache_stats.misses;
                if total > 0 {
                    log::info!(
                        "[DIAG] GlyphCache: size={} hits={} misses={} hit_rate={:.1}% evictions={}",
                        cache_size,
                        cache_stats.hits,
                        cache_stats.misses,
                        (cache_stats.hits as f64 / total as f64) * 100.0,
                        cache_stats.evictions
                    );
                }
            }

            if stats.total_frames % 300 == 0 && log::log_enabled!(log::Level::Debug) {
                report.print();
            }
        }
    }
}
