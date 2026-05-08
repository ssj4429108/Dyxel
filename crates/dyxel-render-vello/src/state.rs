// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! State owner groups for `VelloBackend`.
//!
//! These structs intentionally only group existing fields. They do not change
//! algorithms or lock behavior; the next cleanup steps can move methods behind
//! these owners one responsibility at a time.

use crate::blur::types::{BackdropBlurTexture, BlurAtlasTexture, BlurredTextureEntry};
use crate::cache::CachedDraw;
use crate::frame::{TripleBuffer, TripleBufferSlot};
use crate::shadow::{ShadowCacheEntry, ShadowCacheStats};
use crate::text::{GlyphRunCacheEntry, GlyphRunCacheKey, GlyphRunCacheStats};
use crate::AsyncShared;
use dyxel_perf::{PerfConfig, PerformanceDiagnostics, PerformanceMonitor, SharedPerfMonitor};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use dyxel_render_api::SharedMutex;
use kurbo::Affine;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::Arc;
use vello::{peniko, wgpu, Renderer, Scene};

pub(crate) struct RendererState {
    renderer: AsyncShared<Option<Renderer>>,
    pipeline_cache: AsyncShared<Option<wgpu::PipelineCache>>,
    cache_path: AsyncShared<Option<String>>,
    cache_saved: AtomicBool,
    cache_stage: AsyncShared<Option<u8>>,
    init_device_info: SharedMutex<Option<(String, Option<wgpu::PipelineCache>, Option<u8>)>>,
    memory_optimizer: SharedMutex<dyxel_perf::MemoryOptimizer>,
    is_loading: Arc<AtomicBool>,
    loading_handle: SharedMutex<Option<std::thread::JoinHandle<()>>>,
    renderer_id: Arc<AtomicU64>,
}

impl RendererState {
    pub(crate) fn new(memory_optimizer: dyxel_perf::MemoryOptimizer) -> Self {
        Self {
            renderer: AsyncShared::new(std::sync::Mutex::new(None)),
            pipeline_cache: AsyncShared::new(std::sync::Mutex::new(None)),
            cache_path: AsyncShared::new(std::sync::Mutex::new(None)),
            cache_saved: AtomicBool::new(false),
            cache_stage: AsyncShared::new(std::sync::Mutex::new(None)),
            init_device_info: SharedMutex::new(None),
            memory_optimizer: SharedMutex::new(memory_optimizer),
            is_loading: Arc::new(AtomicBool::new(false)),
            loading_handle: SharedMutex::new(None),
            renderer_id: Arc::new(AtomicU64::new(1)),
        }
    }

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

    pub(crate) fn renderer_handle(&self) -> &AsyncShared<Option<Renderer>> {
        &self.renderer
    }

    pub(crate) fn renderer_lock(&self) -> std::sync::MutexGuard<'_, Option<Renderer>> {
        self.renderer.lock().unwrap()
    }

    pub(crate) fn is_renderer_ready(&self) -> bool {
        self.renderer.lock().unwrap().is_some()
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.is_loading.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn set_loading(&self, loading: bool) {
        self.is_loading
            .store(loading, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn take_deferred_init_info(
        &self,
    ) -> Option<(String, Option<wgpu::PipelineCache>, Option<u8>)> {
        self.init_device_info.lock().unwrap().take()
    }

    pub(crate) fn restore_pipeline_cache_if_missing(
        &self,
        pipeline_cache: &Option<wgpu::PipelineCache>,
    ) {
        let mut stored_cache = self.pipeline_cache.lock().unwrap();
        if stored_cache.is_none() && pipeline_cache.is_some() {
            log::warn!(
                "[ColdStart] renderer pipeline cache was None in ensure_renderer_initialized_async; restoring from init_device_info"
            );
            *stored_cache = pipeline_cache.clone();
        }
    }

    pub(crate) fn memory_tier(&self) -> dyxel_perf::DeviceMemoryTier {
        self.memory_optimizer.lock().unwrap().tier()
    }

    pub(crate) fn renderer_handle_clone(&self) -> AsyncShared<Option<Renderer>> {
        self.renderer.clone()
    }

    pub(crate) fn renderer_id_handle(&self) -> Arc<AtomicU64> {
        self.renderer_id.clone()
    }

    pub(crate) fn current_renderer_id(&self) -> u64 {
        self.renderer_id.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn loading_flag_handle(&self) -> Arc<AtomicBool> {
        self.is_loading.clone()
    }

    pub(crate) fn pipeline_cache_handle(&self) -> AsyncShared<Option<wgpu::PipelineCache>> {
        self.pipeline_cache.clone()
    }

    pub(crate) fn cache_path_handle(&self) -> AsyncShared<Option<String>> {
        self.cache_path.clone()
    }

    pub(crate) fn cache_stage_handle(&self) -> AsyncShared<Option<u8>> {
        self.cache_stage.clone()
    }

    pub(crate) fn set_loading_handle(&self, handle: std::thread::JoinHandle<()>) {
        *self.loading_handle.lock().unwrap() = Some(handle);
    }

    pub(crate) fn cache_already_saved(&self) -> bool {
        self.cache_saved.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn mark_cache_saved(&self) {
        self.cache_saved
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn store_cache_info(
        &self,
        cache_path: String,
        pipeline_cache: Option<wgpu::PipelineCache>,
        cache_stage: Option<u8>,
    ) {
        *self.pipeline_cache.lock().unwrap() = pipeline_cache;
        *self.cache_path.lock().unwrap() = Some(cache_path);
        *self.cache_stage.lock().unwrap() = cache_stage;
    }

    pub(crate) fn store_deferred_init_info(
        &self,
        cache_path: String,
        pipeline_cache: Option<wgpu::PipelineCache>,
        cache_stage: Option<u8>,
    ) {
        *self.init_device_info.lock().unwrap() = Some((cache_path, pipeline_cache, cache_stage));
    }

    pub(crate) fn with_pipeline_cache<R>(
        &self,
        f: impl FnOnce(Option<&wgpu::PipelineCache>) -> R,
    ) -> R {
        let pipeline_cache = self.pipeline_cache.lock().unwrap();
        f(pipeline_cache.as_ref())
    }

    pub(crate) fn initialize_memory_optimizer(&self) {
        let memory_optimizer = self.memory_optimizer.lock().unwrap();
        memory_optimizer.initialize();
        log::info!(
            "[Memory] Initialized memory optimizer for tier: {:?}",
            memory_optimizer.tier()
        );
    }

    #[inline]
    pub(crate) fn render_scene_to_texture(
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
                    base_color: peniko::Color::TRANSPARENT,
                    width: w,
                    height: h,
                    antialiasing_method: aa_config,
                },
            )
            .map_err(|e| anyhow::anyhow!("render_to_texture failed: {:?}", e))
    }
}

pub(crate) struct BlitFrameSlot {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) bind_group: wgpu::BindGroup,
}

pub(crate) struct BlitState {
    blit_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    sampler: SharedMutex<Option<wgpu::Sampler>>,
    blit_shader: SharedMutex<Option<wgpu::ShaderModule>>,
    blit_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    blit_pipeline_format: SharedMutex<Option<wgpu::TextureFormat>>,
    triple_buffer: SharedMutex<Option<TripleBuffer>>,
    children_blit_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
}

impl BlitState {
    pub(crate) fn new() -> Self {
        Self {
            blit_bind_group_layout: SharedMutex::new(None),
            sampler: SharedMutex::new(None),
            blit_shader: SharedMutex::new(None),
            blit_pipeline: SharedMutex::new(None),
            blit_pipeline_format: SharedMutex::new(None),
            triple_buffer: SharedMutex::new(None),
            children_blit_pipeline: SharedMutex::new(None),
        }
    }

    /// Create blit shader, bind group layout, and sampler for the blit pipeline.
    pub(crate) fn create_resources(
        device: &wgpu::Device,
    ) -> (wgpu::ShaderModule, wgpu::BindGroupLayout, wgpu::Sampler) {
        let blit_shader = if cfg!(target_os = "android") {
            let spv_words: Vec<u32> = crate::BLIT_SHADER_SPV
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

        (blit_shader, blit_bl, sampler)
    }

    pub(crate) fn set_resources(
        &self,
        blit_shader: wgpu::ShaderModule,
        blit_bind_group_layout: wgpu::BindGroupLayout,
        sampler: wgpu::Sampler,
    ) {
        *self.blit_shader.lock().unwrap() = Some(blit_shader);
        *self.blit_bind_group_layout.lock().unwrap() = Some(blit_bind_group_layout);
        *self.sampler.lock().unwrap() = Some(sampler);
    }

    pub(crate) fn create_surface_pipelines(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
        let blit_layout_lock = self.blit_bind_group_layout.lock().unwrap();
        let blit_shader_lock = self.blit_shader.lock().unwrap();
        let layout = blit_layout_lock
            .as_ref()
            .expect("blit bind group layout should be initialized");
        let shader = blit_shader_lock
            .as_ref()
            .expect("blit shader should be initialized");

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[layout],
            push_constant_ranges: &[],
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: pipeline_cache,
        });

        let children_blit_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Children Blit Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: pipeline_cache,
            });

        (blit_pipeline, children_blit_pipeline)
    }

    pub(crate) fn set_surface_pipelines(
        &self,
        blit_pipeline: wgpu::RenderPipeline,
        children_blit_pipeline: wgpu::RenderPipeline,
    ) {
        *self.blit_pipeline.lock().unwrap() = Some(blit_pipeline);
        *self.children_blit_pipeline.lock().unwrap() = Some(children_blit_pipeline);
    }

    /// Prewarm the main blit pipeline for a target format.
    pub(crate) fn prewarm_pipeline(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) {
        let blit_shader = self.blit_shader.lock().unwrap();
        let blit_layout = self.blit_bind_group_layout.lock().unwrap();

        if let (Some(shader), Some(layout)) = (&*blit_shader, &*blit_layout) {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Blit Pipeline Layout Prewarm"),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Blit Pipeline Prewarm"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: pipeline_cache,
            });
            *self.blit_pipeline.lock().unwrap() = Some(pipeline);
            *self.blit_pipeline_format.lock().unwrap() = Some(format);
        }
    }

    pub(crate) fn ensure_pipeline(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) {
        let needs_recreate = {
            let format_guard = self.blit_pipeline_format.lock().unwrap();
            self.blit_pipeline.lock().unwrap().is_none() || *format_guard != Some(format)
        };
        if needs_recreate {
            self.prewarm_pipeline(device, format, pipeline_cache);
        }
    }

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

    pub(crate) fn sampler_clone(&self) -> Option<wgpu::Sampler> {
        self.sampler.lock().unwrap().clone()
    }

    pub(crate) fn current_triple_buffer_slot(&self) -> Option<BlitFrameSlot> {
        let triple_buffer = self.triple_buffer.lock().unwrap();
        let slot = triple_buffer.as_ref()?.current();
        Some(BlitFrameSlot {
            texture: slot.texture.clone(),
            view: slot.view.clone(),
            bind_group: slot.bind_group.clone(),
        })
    }

    /// Draw a fullscreen blit triangle using an already-created texture bind
    /// group. The caller owns the surrounding render pass lifetime.
    pub(crate) fn draw_bind_group(
        &self,
        rp: &mut wgpu::RenderPass<'_>,
        bind_group: &wgpu::BindGroup,
    ) {
        let blit_pipeline_guard = self.blit_pipeline.lock().unwrap();
        let blit_pipeline = blit_pipeline_guard
            .as_ref()
            .expect("blit pipeline should be initialized");
        rp.set_pipeline(blit_pipeline);
        rp.set_bind_group(0, bind_group, &[]);
        rp.draw(0..3, 0..1);
    }

    /// Create a bind group for a texture view using the main blit layout and
    /// sampler. Used by debug/capture blits that are not part of the triple
    /// buffer ring.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn create_texture_bind_group(
        &self,
        device: &wgpu::Device,
        label: &'static str,
        texture_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let layout_guard = self.blit_bind_group_layout.lock().unwrap();
        let layout = layout_guard
            .as_ref()
            .expect("blit bind group layout should be initialized");
        let sampler_guard = self.sampler.lock().unwrap();
        let sampler = sampler_guard
            .as_ref()
            .expect("blit sampler should be initialized");
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }
}

pub(crate) struct BlurState {
    pub(crate) filter_pipeline: SharedMutex<Option<crate::filter_pipeline::FilterPipeline>>,
    pub(crate) blur_composite_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    pub(crate) blur_composite_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    pub(crate) blur_composite_uniforms: SharedMutex<Option<wgpu::Buffer>>,
    pub(crate) blur_composite_overlay_uniforms: SharedMutex<Option<wgpu::Buffer>>,
    pub(crate) blur_instanced_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    pub(crate) blur_instanced_pipeline_format: SharedMutex<Option<wgpu::TextureFormat>>,
    pub(crate) blur_instanced_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    pub(crate) blur_instanced_bind_group: SharedMutex<Option<wgpu::BindGroup>>,
    pub(crate) blur_instance_buffer: SharedMutex<Option<wgpu::Buffer>>,
    pub(crate) blur_instance_capacity: SharedMutex<usize>,
    pub(crate) blur_frame_uniform: SharedMutex<Option<wgpu::Buffer>>,
    pub(crate) blur_staging_buffer: SharedMutex<Option<wgpu::Buffer>>,
    pub(crate) blur_staging_alignment: SharedMutex<usize>,
    pub(crate) blur_staging_offset: AtomicUsize,
    pub(crate) blurred_textures: SharedMutex<Vec<BlurredTextureEntry>>,
    pub(crate) backdrop_blur: SharedMutex<Option<BackdropBlurTexture>>,
    pub(crate) blur_atlas: SharedMutex<Option<BlurAtlasTexture>>,
    pub(crate) blur_source_atlas: SharedMutex<Option<BlurAtlasTexture>>,
    pub(crate) blur_atlas_wide_active_last_frame: AtomicBool,
    pub(crate) texture_pool: SharedMutex<Option<crate::texture_pool::SharedTexturePool>>,
}

impl BlurState {
    pub(crate) fn new() -> Self {
        Self {
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
            blur_staging_offset: AtomicUsize::new(0),
            blurred_textures: SharedMutex::new(Vec::new()),
            backdrop_blur: SharedMutex::new(None),
            blur_atlas: SharedMutex::new(None),
            blur_source_atlas: SharedMutex::new(None),
            blur_atlas_wide_active_last_frame: AtomicBool::new(false),
            texture_pool: SharedMutex::new(None),
        }
    }

    pub(crate) fn collect_returned_textures_and_reset_staging(&self) {
        if let Some(ref pool) = *self.texture_pool.lock().unwrap() {
            pool.collect_returns();
        }
        self.blur_staging_offset
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn set_filter_pipeline(&self, pipeline: crate::filter_pipeline::FilterPipeline) {
        *self.filter_pipeline.lock().unwrap() = Some(pipeline);
    }

    pub(crate) fn set_texture_pool(&self, pool: crate::texture_pool::SharedTexturePool) {
        *self.texture_pool.lock().unwrap() = Some(pool);
    }

    pub(crate) fn has_blur_entries(&self) -> bool {
        !self.blurred_textures.lock().unwrap().is_empty()
    }

    /// Borrow the current blur scene-building state, run the scene traversal,
    /// then prune entries that were not seen this frame.
    ///
    /// This keeps blur-entry lifetime management inside `BlurState` while the
    /// caller still owns the actual recursive scene traversal.
    pub(crate) fn with_scene_entries<R>(
        &self,
        build: impl FnOnce(
            Option<&crate::filter_pipeline::FilterPipeline>,
            &mut Vec<BlurredTextureEntry>,
            u64,
        ) -> R,
    ) -> R {
        let blur_scene_frame =
            crate::BLUR_SCENE_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let filter_pipeline = self.filter_pipeline.lock().unwrap();
        let mut blurred_textures = self.blurred_textures.lock().unwrap();

        let result = build(
            filter_pipeline.as_ref(),
            &mut blurred_textures,
            blur_scene_frame,
        );
        blurred_textures.retain(|entry| entry.last_seen_frame == blur_scene_frame);
        result
    }
}

pub(crate) struct RasterCacheState {
    cached_textures:
        SharedMutex<std::collections::HashMap<u32, dyxel_render_api::raster_cache::TextureId>>,
    gpu_texture_pool: SharedMutex<Option<crate::texture_pool::GpuTexturePool>>,
}

#[derive(Clone, Copy)]
pub(crate) struct RasterCacheLookup<'a> {
    cached_textures: &'a std::collections::HashMap<u32, dyxel_render_api::raster_cache::TextureId>,
}

impl RasterCacheLookup<'_> {
    #[inline]
    pub(crate) fn try_emit_cached_draw(
        &self,
        node_id: u32,
        transform: Affine,
        width: f32,
        height: f32,
        cached_draws: &mut Vec<CachedDraw>,
    ) -> bool {
        if let Some(&texture_id) = self.cached_textures.get(&node_id) {
            cached_draws.push(CachedDraw {
                texture_id: crate::texture_pool::TextureId(texture_id.0),
                transform,
                width,
                height,
            });
            true
        } else {
            false
        }
    }
}

impl RasterCacheState {
    pub(crate) fn new() -> Self {
        Self {
            cached_textures: SharedMutex::new(std::collections::HashMap::new()),
            gpu_texture_pool: SharedMutex::new(None),
        }
    }

    pub(crate) fn recycle_plans(&self, plans: &[dyxel_render_api::RecyclePlan]) {
        let mut cached_textures = self.cached_textures.lock().unwrap();
        let mut gpu_texture_pool = self.gpu_texture_pool.lock().unwrap();
        if let Some(pool) = gpu_texture_pool.as_mut() {
            for plan in plans {
                pool.release(crate::texture_pool::TextureId(plan.texture_id.0));
                cached_textures.remove(&plan.node_id);
            }
        }
    }

    pub(crate) fn acquire_bake_target(
        &self,
        width: u32,
        height: u32,
    ) -> Option<(crate::texture_pool::TextureId, wgpu::TextureView)> {
        let mut gpu_texture_pool = self.gpu_texture_pool.lock().unwrap();
        let pool = gpu_texture_pool.as_mut()?;
        let texture_id = pool.acquire(width, height, wgpu::TextureFormat::Rgba8Unorm);
        let target_view = pool.get_texture(texture_id)?.view().clone();
        Some((texture_id, target_view))
    }

    pub(crate) fn record_bake(&self, node_id: u32, texture_id: crate::texture_pool::TextureId) {
        self.cached_textures.lock().unwrap().insert(
            node_id,
            dyxel_render_api::raster_cache::TextureId(texture_id.0),
        );
    }

    pub(crate) fn with_cached_lookup<R>(&self, f: impl FnOnce(RasterCacheLookup<'_>) -> R) -> R {
        let cached_textures = self.cached_textures.lock().unwrap();
        f(RasterCacheLookup {
            cached_textures: &cached_textures,
        })
    }

    pub(crate) fn set_gpu_texture_pool(&self, pool: crate::texture_pool::GpuTexturePool) {
        *self.gpu_texture_pool.lock().unwrap() = Some(pool);
    }

    pub(crate) fn with_gpu_texture_pool<R>(
        &self,
        f: impl FnOnce(Option<&crate::texture_pool::GpuTexturePool>) -> R,
    ) -> R {
        let gpu_texture_pool = self.gpu_texture_pool.lock().unwrap();
        f(gpu_texture_pool.as_ref())
    }

    pub(crate) fn execute_bake_plans(
        &self,
        package: &dyxel_render_api::RenderPackage,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
        mut build_scene: impl FnMut(u32, &mut Scene, RasterCacheLookup<'_>, &mut Renderer),
    ) {
        self.recycle_plans(&package.recycle_plans);

        // LIMIT: process at most 2 bake plans per frame to prevent render time spikes.
        const MAX_BAKES_PER_FRAME: usize = 2;
        for plan in package.bake_plans.iter().take(MAX_BAKES_PER_FRAME) {
            let tex_w = plan.width;
            let tex_h = plan.height;
            if tex_w == 0 || tex_h == 0 {
                continue;
            }

            if let Some((texture_id, target_view)) = self.acquire_bake_target(tex_w, tex_h) {
                let mut bake_scene = Scene::new();
                self.with_cached_lookup(|cached_lookup| {
                    build_scene(plan.node_id, &mut bake_scene, cached_lookup, renderer);
                });

                let _ = renderer.render_to_texture(
                    device,
                    queue,
                    &bake_scene,
                    &target_view,
                    &vello::RenderParams {
                        base_color: peniko::Color::TRANSPARENT,
                        width: tex_w,
                        height: tex_h,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                );
                self.record_bake(plan.node_id, texture_id);
            }
        }
    }
}

pub(crate) struct ShadowCacheState {
    shadow_cache:
        SharedMutex<std::collections::HashMap<crate::shadow::ShadowCacheKey, ShadowCacheEntry>>,
    shadow_cache_stats: SharedMutex<ShadowCacheStats>,
    shadow_cache_misses_this_frame: AtomicU64,
    shadow_cache_renderer_id: AtomicU64,
}

impl ShadowCacheState {
    pub(crate) fn new() -> Self {
        Self {
            shadow_cache: SharedMutex::new(std::collections::HashMap::new()),
            shadow_cache_stats: SharedMutex::new(ShadowCacheStats::default()),
            shadow_cache_misses_this_frame: AtomicU64::new(0),
            shadow_cache_renderer_id: AtomicU64::new(0),
        }
    }

    pub(crate) fn reset_frame_budget(&self) {
        self.shadow_cache_misses_this_frame
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn evict_stale_entries(&self, renderer: &mut Renderer, current_frame: u64) {
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

    pub(crate) fn sync_renderer_id(&self, renderer: &mut Renderer, current_id: u64) {
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
    pub(crate) fn draw_node_shadow(
        &self,
        id: u32,
        shadow: &dyxel_render_api::ShadowDesc,
        scene: &mut Scene,
        local_transform: Affine,
        node_width: f64,
        node_height: f64,
        border_radius: f64,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
    ) {
        crate::shadow::draw_node_shadow(
            id,
            shadow,
            scene,
            local_transform,
            node_width,
            node_height,
            border_radius,
            device,
            queue,
            renderer,
            crate::shadow::ShadowCacheRefs {
                cache: &self.shadow_cache,
                stats: &self.shadow_cache_stats,
                misses_this_frame: &self.shadow_cache_misses_this_frame,
            },
        );
    }
}

pub(crate) struct TextCacheState {
    glyph_run_cache: SharedMutex<std::collections::HashMap<GlyphRunCacheKey, GlyphRunCacheEntry>>,
    glyph_run_cache_stats: SharedMutex<GlyphRunCacheStats>,
}

impl TextCacheState {
    pub(crate) fn new() -> Self {
        Self {
            glyph_run_cache: SharedMutex::new(std::collections::HashMap::new()),
            glyph_run_cache_stats: SharedMutex::new(GlyphRunCacheStats::default()),
        }
    }

    pub(crate) fn evict_stale_glyph_runs(&self, current_frame: u64) {
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
    pub(crate) fn draw_prepared_text(
        &self,
        scene: &mut Scene,
        payload: &dyxel_render_api::TextDrawPayload,
        local_transform: Affine,
        opacity: f32,
    ) {
        crate::text::draw_prepared_text(
            scene,
            payload,
            local_transform,
            &self.glyph_run_cache,
            &self.glyph_run_cache_stats,
            opacity,
        );
    }
}

pub(crate) struct DiagnosticsState {
    perf_monitor: SharedPerfMonitor,
    #[allow(dead_code)]
    diagnostics: SharedMutex<Option<PerformanceDiagnostics>>,
    pacer_wait_ms: SharedMutex<f64>,
    frame_interval_ms: SharedMutex<f64>,
    frame_perf_stats: SharedMutex<dyxel_perf::FramePerformanceStats>,
}

impl DiagnosticsState {
    pub(crate) fn new(perf_config: PerfConfig) -> Self {
        Self {
            perf_monitor: Arc::new(std::sync::Mutex::new(PerformanceMonitor::new(perf_config))),
            diagnostics: SharedMutex::new(Some(PerformanceDiagnostics::new(120))),
            pacer_wait_ms: SharedMutex::new(0.0),
            frame_interval_ms: SharedMutex::new(0.0),
            frame_perf_stats: SharedMutex::new(dyxel_perf::FramePerformanceStats::default()),
        }
    }

    pub(crate) fn perf_monitor_handle(&self) -> SharedPerfMonitor {
        self.perf_monitor.clone()
    }

    pub(crate) fn toggle_perf_overlay(&self) {
        self.perf_monitor.lock().unwrap().toggle_overlay();
    }

    pub(crate) fn disable_perf_overlay(&self) {
        let mut monitor = self.perf_monitor.lock().unwrap();
        if monitor.should_show_overlay() {
            monitor.toggle_overlay();
        }
    }

    pub(crate) fn begin_frame(&self) {
        self.perf_monitor.lock().unwrap().begin_frame();
    }

    pub(crate) fn set_frame_timing(&self, pacer_wait_ms: f64, frame_interval_ms: f64) {
        *self.pacer_wait_ms.lock().unwrap() = pacer_wait_ms;
        *self.frame_interval_ms.lock().unwrap() = frame_interval_ms;
    }

    pub(crate) fn set_frame_performance_stats(&self, stats: dyxel_perf::FramePerformanceStats) {
        *self.frame_perf_stats.lock().unwrap() = stats;
    }

    pub(crate) fn log_frame_diagnostics(
        &self,
        shadow: &ShadowCacheState,
        text: &TextCacheState,
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

            if stats.total_frames % crate::DIAG_LOG_EVERY_N_FRAMES == 0 || total > 18.0 {
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
                let cache_stats = shadow.shadow_cache_stats.lock().unwrap();
                let cache_size = shadow.shadow_cache.lock().unwrap().len();
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
                let cache_stats = text.glyph_run_cache_stats.lock().unwrap();
                let cache_size = text.glyph_run_cache.lock().unwrap().len();
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
