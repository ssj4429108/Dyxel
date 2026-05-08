// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Blur GPU pipeline setup: composite pipeline, atlas textures, and instanced resources.

use super::atlas::{
    blur_atlas_wide_layout_within_budget, compute_blur_atlas_layout, BLUR_ATLAS_LEGACY_GAP_PX,
    BLUR_ATLAS_WIDE_GAP_PX, USE_ATLAS_WIDE_BACKDROP_BLUR,
};
use super::atlas_pass::pack_blur_atlas;
use super::dirty::{
    collect_blur_dirty_report, kawase_pass_class_for_radius, log_blur_dirty_report,
    USE_FULL_FRAME_BACKDROP_BLUR,
};
use super::passes::{
    apply_legacy_kawase_blur, copy_legacy_blur_sources, select_legacy_rebuild_indices,
};
use super::types::{
    BackdropBlurTexture, BlurAtlasTexture, BlurDirtyKind, BlurFrameUniform, BlurInstance,
    BlurredTextureEntry,
};
use crate::{state::BlurState, DIAG_LOG_EVERY_N_FRAMES, FRAME_COUNTER};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;

impl BlurState {
    #[inline]
    pub(crate) fn create_blur_composite_pipeline(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) {
        // Create bind group layout with uniform buffer for transform and overlay
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blur Composite Bind Group Layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create uniform buffer (3 rows of vec4 = 48 bytes)
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blur Composite Uniform Buffer"),
            size: 48, // 3 * 16 bytes (aligned vec4s)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create overlay uniform buffer (color + radius + size + source rect = 48 bytes)
        let overlay_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blur Overlay Uniform Buffer"),
            size: 48, // 3 * 16 bytes (aligned vec4s)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Load shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blur Composite Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../blur_composite.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blur Composite Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blur Composite Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: format,
                    // Premultiplied alpha blending: shader outputs premultiplied colors
                    // src_factor=One because RGB is already multiplied by alpha
                    // This correctly composites frosted glass over the main scene
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        *self.blur_composite_pipeline.lock().unwrap() = Some(pipeline);
        *self.blur_composite_bind_group_layout.lock().unwrap() = Some(bind_group_layout);
        *self.blur_composite_uniforms.lock().unwrap() = Some(uniform_buffer);
        *self.blur_composite_overlay_uniforms.lock().unwrap() = Some(overlay_uniform_buffer);

        // Initialize 1MB staging buffer for zero-copy blur uniform updates
        let alignment = device.limits().min_uniform_buffer_offset_alignment as usize;
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blur Staging Buffer"),
            size: 1024 * 1024,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        *self.blur_staging_buffer.lock().unwrap() = Some(staging_buffer);
        *self.blur_staging_alignment.lock().unwrap() = alignment;

        log::debug!("[Blur] Composite pipeline initialized");
    }

    #[inline]
    pub(crate) fn ensure_backdrop_blur_texture(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) {
        let mut backdrop = self.backdrop_blur.lock().unwrap();
        let needs_create = backdrop
            .as_ref()
            .map_or(true, |tex| tex.width != width || tex.height != height);

        if needs_create {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Backdrop Blur Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            *backdrop = Some(BackdropBlurTexture {
                texture,
                view,
                width,
                height,
            });
            *self.blur_instanced_bind_group.lock().unwrap() = None;
        }
    }

    #[inline]
    pub(crate) fn ensure_blur_atlas_texture(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> bool {
        let mut atlas = self.blur_atlas.lock().unwrap();
        let needs_create = atlas
            .as_ref()
            .map_or(true, |tex| tex.width != width || tex.height != height);
        if needs_create {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Blur Legacy Atlas Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            *atlas = Some(BlurAtlasTexture {
                texture,
                view,
                width,
                height,
            });
            *self.blur_instanced_bind_group.lock().unwrap() = None;
        }
        needs_create
    }

    #[inline]
    pub(crate) fn ensure_blur_source_atlas_texture(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> bool {
        let mut atlas = self.blur_source_atlas.lock().unwrap();
        let needs_create = atlas
            .as_ref()
            .map_or(true, |tex| tex.width != width || tex.height != height);
        if needs_create {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Blur Raw Source Atlas Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            *atlas = Some(BlurAtlasTexture {
                texture,
                view,
                width,
                height,
            });
        }
        needs_create
    }

    #[inline]
    pub(crate) fn ensure_blur_instanced_resources(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        instance_count: usize,
    ) {
        let pipeline_needs_create = {
            let pipeline = self.blur_instanced_pipeline.lock().unwrap();
            let pipeline_format = self.blur_instanced_pipeline_format.lock().unwrap();
            pipeline.is_none() || pipeline_format.map_or(true, |f| f != format)
        };

        if pipeline_needs_create {
            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Blur Instanced Bind Group Layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blur Atlas Instanced Composite Shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../blur_atlas_instanced_composite.wgsl").into(),
                ),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Blur Instanced Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Blur Instanced Composite Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

            *self.blur_instanced_pipeline.lock().unwrap() = Some(pipeline);
            *self.blur_instanced_pipeline_format.lock().unwrap() = Some(format);
            *self.blur_instanced_bind_group_layout.lock().unwrap() = Some(bind_group_layout);
            *self.blur_instanced_bind_group.lock().unwrap() = None;
        }

        let frame_needs_create = self.blur_frame_uniform.lock().unwrap().is_none();
        if frame_needs_create {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Blur Frame Uniform"),
                size: std::mem::size_of::<BlurFrameUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            *self.blur_frame_uniform.lock().unwrap() = Some(buffer);
            *self.blur_instanced_bind_group.lock().unwrap() = None;
        }

        let required_capacity = instance_count.max(1).next_power_of_two();
        let mut capacity = self.blur_instance_capacity.lock().unwrap();
        if self.blur_instance_buffer.lock().unwrap().is_none() || *capacity < required_capacity {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Blur Instance Buffer"),
                size: (required_capacity * std::mem::size_of::<BlurInstance>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            *self.blur_instance_buffer.lock().unwrap() = Some(buffer);
            *self.blur_instanced_bind_group.lock().unwrap() = None;
            *capacity = required_capacity;
        }
    }

    /// Try the atlas-wide blur path: pack all blur entries into a single atlas,
    /// copy scene regions, and run a single Kawase pass. Returns
    /// `(atlas_wide_valid, source_copy_count)` on success, or `(false, 0)` if the
    /// path is not applicable.
    #[inline]
    pub(crate) fn try_atlas_wide_blur(
        &self,
        device: &wgpu::Device,
        post_enc: &mut wgpu::CommandEncoder,
        pipeline: &crate::filter_pipeline::FilterPipeline,
        scene_texture: &wgpu::Texture,
        blurred_textures: &mut [BlurredTextureEntry],
        w: u32,
        h: u32,
        max_radius: f32,
        current_frame: u64,
        stage_timer: &mut dyxel_perf::FrameTimer,
    ) -> (bool, usize) {
        let layout = compute_blur_atlas_layout(blurred_textures, w, h, BLUR_ATLAS_WIDE_GAP_PX);
        let radius_class_reference = layout.as_ref().and_then(|layout| {
            layout
                .placements
                .first()
                .map(|(idx, _, _)| kawase_pass_class_for_radius(blurred_textures[*idx].blur_radius))
        });
        let radius_class_uniform =
            if let (Some(layout), Some(reference)) = (layout.as_ref(), radius_class_reference) {
                layout.placements.iter().all(|(idx, _, _)| {
                    kawase_pass_class_for_radius(blurred_textures[*idx].blur_radius) == reference
                })
            } else {
                false
            };

        let layout = match layout {
            Some(l) => l,
            None => return (false, 0),
        };

        let atlas_wide_within_budget = blur_atlas_wide_layout_within_budget(&layout);
        if layout.placements.len() < 8 || !radius_class_uniform || !atlas_wide_within_budget {
            if current_frame % DIAG_LOG_EVERY_N_FRAMES == 0 {
                log::info!(
                    "[BlurAtlasWide] fallback: placements={} radius_class_uniform={} budget_ok={} atlas={}x{}",
                    layout.placements.len(),
                    radius_class_uniform,
                    atlas_wide_within_budget,
                    layout.width,
                    layout.height
                );
            }
            return (false, 0);
        }

        self.ensure_blur_atlas_texture(device, layout.width, layout.height);
        self.ensure_blur_source_atlas_texture(device, layout.width, layout.height);
        let source_atlas_texture = {
            let guard = self.blur_source_atlas.lock().unwrap();
            guard.as_ref().map(|atlas| atlas.texture.clone())
        };
        let blurred_atlas_texture = {
            let guard = self.blur_atlas.lock().unwrap();
            guard.as_ref().map(|atlas| atlas.texture.clone())
        };

        let (source_atlas_texture, blurred_atlas_texture) =
            match (source_atlas_texture, blurred_atlas_texture) {
                (Some(s), Some(b)) => (s, b),
                _ => return (false, 0),
            };

        post_enc.clear_texture(
            &source_atlas_texture,
            &wgpu::ImageSubresourceRange {
                aspect: wgpu::TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: None,
                base_array_layer: 0,
                array_layer_count: None,
            },
        );

        let mut copied_indices = Vec::with_capacity(layout.placements.len());
        for &(idx, ax, ay) in &layout.placements {
            let entry = &mut blurred_textures[idx];
            let (src_x, src_y, src_w, src_h) = entry.source_rect;
            let padding = ((entry.width as f32 - src_w) * 0.5).max(0.0) as u32;

            #[cfg(target_os = "android")]
            let src_origin_y = (h as f32 - src_y - src_h).max(0.0) as u32;
            #[cfg(not(target_os = "android"))]
            let src_origin_y = src_y.max(0.0) as u32;

            let src_origin_x = src_x.max(0.0) as u32;
            let copy_width = (src_w as u32)
                .min(w.saturating_sub(src_origin_x))
                .min(entry.width.saturating_sub(padding));
            let copy_height = (src_h as u32)
                .min(h.saturating_sub(src_origin_y))
                .min(entry.height.saturating_sub(padding));
            if copy_width == 0 || copy_height == 0 {
                entry.blur_valid = false;
                entry.blur_rebuild_pending = true;
                entry.atlas_valid = false;
                entry.atlas_dirty = true;
                continue;
            }

            post_enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: scene_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: src_origin_x,
                        y: src_origin_y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &source_atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: ax + padding,
                        y: ay + padding,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: copy_width,
                    height: copy_height,
                    depth_or_array_layers: 1,
                },
            );
            copied_indices.push((idx, ax, ay));
        }
        let source_copies = copied_indices.len();
        stage_timer.mark("blur_copy_submit");

        let mut atlas_wide_valid = false;
        if !copied_indices.is_empty() {
            let result = pipeline.apply_frosted_glass_kawase(
                post_enc,
                &source_atlas_texture,
                &blurred_atlas_texture,
                max_radius,
                None,
            );
            if let Err(e) = result {
                log::warn!("[BlurAtlasWide] atlas-wide Kawase failed: {:?}", e);
            } else {
                for (idx, ax, ay) in copied_indices {
                    if let Some(entry) = blurred_textures.get_mut(idx) {
                        entry.blur_valid = true;
                        entry.blur_rebuild_pending = false;
                        entry.atlas_valid = true;
                        entry.atlas_dirty = false;
                        entry.atlas_x = ax;
                        entry.atlas_y = ay;
                        entry.last_blur_rebuild_frame = current_frame;
                        if entry.dirty_kind != BlurDirtyKind::ChildrenChanged {
                            entry.dirty_kind = BlurDirtyKind::Clean;
                        }
                    }
                }
                for entry in blurred_textures.iter_mut() {
                    if entry.blur_rebuild_pending {
                        continue;
                    }
                    if matches!(
                        entry.dirty_kind,
                        BlurDirtyKind::OverlayOnlyChanged | BlurDirtyKind::Clean
                    ) {
                        entry.dirty_kind = BlurDirtyKind::Clean;
                    }
                }
                atlas_wide_valid = true;
            }
        }
        stage_timer.mark("blur_render_submit");

        if atlas_wide_valid && current_frame % DIAG_LOG_EVERY_N_FRAMES == 0 {
            log::info!(
                "[BlurAtlasWide] Frame {} — copied {} slots, atlas={}x{} slot={} gap={} radius={:.1}",
                current_frame,
                source_copies,
                layout.width,
                layout.height,
                layout.slot,
                layout.gap,
                max_radius,
            );
        }

        (atlas_wide_valid, source_copies)
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
        sampler: Option<&wgpu::Sampler>,
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
                    sampler,
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
}
