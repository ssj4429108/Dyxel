// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pass 4: Blur compositing — draws blurred textures, cached subtrees,
//! and children overlays onto the final render target.

use super::dirty::{blur_entry_visible, USE_FULL_FRAME_BACKDROP_BLUR};
use super::types::BlurredTextureEntry;
use crate::CachedDraw;
use kurbo::Affine;
use std::sync::atomic::AtomicUsize;

/// Pre-locked GPU resources needed by the blur composite pass.
pub(crate) struct BlurCompositeResources<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub layout: &'a wgpu::BindGroupLayout,
    pub sampler: &'a wgpu::Sampler,
    pub staging_buffer: &'a wgpu::Buffer,
    pub staging_alignment: usize,
    pub staging_offset: &'a AtomicUsize,
    pub gpu_texture_pool: Option<&'a crate::texture_pool::GpuTexturePool>,
    pub atlas_pipeline: Option<&'a wgpu::RenderPipeline>,
    pub atlas_bind_group: Option<&'a wgpu::BindGroup>,
    pub atlas_instance_count: u32,
    pub atlas_enabled: bool,
    pub backdrop_view: Option<&'a wgpu::TextureView>,
}

/// Execute Pass 4 blur compositing render commands.
///
/// Draws cached subtrees, atlas-instanced blur, legacy per-entry blur,
/// and children overlays. Returns `true` if any blur textures were composited.
#[inline]
pub(crate) fn composite_blur_pass4(
    rp: &mut wgpu::RenderPass<'_>,
    blurred_textures: &mut [BlurredTextureEntry],
    cached_draws: &[CachedDraw],
    res: &BlurCompositeResources<'_>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    viewport_w: u32,
    viewport_h: u32,
) -> bool {
    let scale_x = 2.0 / viewport_w as f32;
    let scale_y = -2.0 / viewport_h as f32;
    let offset_x = -1.0;
    let offset_y = 1.0;

    // === Draw cached subtrees first (before blur composite) ===
    if !cached_draws.is_empty() {
        log::debug!(
            "[RasterCache] Drawing {} cached subtrees",
            cached_draws.len()
        );
        let stride = res.staging_alignment * 2;

        for draw in cached_draws {
            let affine = draw.transform;
            let mat = affine.as_coeffs();
            let tex_width = draw.width;
            let tex_height = draw.height;

            let uniform_data: [f32; 12] = [
                mat[0] as f32 * tex_width * scale_x,
                mat[2] as f32 * tex_width * scale_x,
                0.0,
                0.0,
                mat[1] as f32 * tex_height * scale_y,
                mat[3] as f32 * tex_height * scale_y,
                0.0,
                0.0,
                mat[4] as f32 * scale_x + offset_x,
                mat[5] as f32 * scale_y + offset_y,
                1.0,
                0.0,
            ];

            let base_offset = res
                .staging_offset
                .fetch_add(stride, std::sync::atomic::Ordering::Relaxed);
            if base_offset + stride > 1024 * 1024 {
                log::warn!("[RasterCache] Staging buffer overflow, skipping remaining draws");
                break;
            }

            let overlay_data: [f32; 12] = [
                0.0, 0.0, 0.0, 0.0, 0.0, tex_width, tex_height, 0.0, 0.0, 0.0, 0.0, 0.0,
            ];

            queue.write_buffer(
                res.staging_buffer,
                base_offset as u64,
                bytemuck::cast_slice(&uniform_data),
            );
            queue.write_buffer(
                res.staging_buffer,
                (base_offset + res.staging_alignment) as u64,
                bytemuck::cast_slice(&overlay_data),
            );

            if let Some(pool) = res.gpu_texture_pool {
                if let Some(ptex) = pool.get_texture(draw.texture_id) {
                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("RasterCache Composite Bind Group"),
                        layout: res.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(ptex.view()),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(res.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: res.staging_buffer,
                                    offset: base_offset as u64,
                                    size: Some(std::num::NonZeroU64::new(48).unwrap()),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: res.staging_buffer,
                                    offset: (base_offset + res.staging_alignment) as u64,
                                    size: Some(std::num::NonZeroU64::new(48).unwrap()),
                                }),
                            },
                        ],
                    });
                    rp.set_pipeline(res.pipeline);
                    rp.set_bind_group(0, &bind_group, &[]);
                    rp.draw(0..6, 0..1);
                }
            }
        }
    }

    // Set pipeline once before the per-entry loop
    rp.set_pipeline(res.pipeline);

    // Atlas instanced draw
    if res.atlas_enabled {
        if let (Some(atlas_pipeline), Some(atlas_bg)) = (res.atlas_pipeline, res.atlas_bind_group) {
            rp.set_pipeline(atlas_pipeline);
            rp.set_bind_group(0, atlas_bg, &[]);
            rp.draw(0..6, 0..res.atlas_instance_count);
            rp.set_pipeline(res.pipeline);
        }
    }

    // Legacy per-entry blur composite + children overlay
    for entry in blurred_textures.iter_mut() {
        let is_visible = blur_entry_visible(entry, viewport_w, viewport_h);
        let blur_drawn_by_atlas = res.atlas_enabled && entry.blur_valid && entry.atlas_valid;
        if !blur_drawn_by_atlas && !entry.skipped_due_to_size && entry.blur_valid && is_visible {
            let affine = entry.transform;
            let mat = affine.as_coeffs();
            let tex_width = entry.width as f32;
            let tex_height = entry.height as f32;

            let uniform_data: [f32; 12] = [
                mat[0] as f32 * tex_width * scale_x,
                mat[2] as f32 * tex_width * scale_x,
                0.0,
                0.0,
                mat[1] as f32 * tex_height * scale_y,
                mat[3] as f32 * tex_height * scale_y,
                0.0,
                0.0,
                mat[4] as f32 * scale_x + offset_x,
                mat[5] as f32 * scale_y + offset_y,
                entry.opacity,
                0.0,
            ];
            let overlay_color = entry.overlay_color;
            // source_width/source_height are zero here, so
            // blur_composite.wgsl samples local texture UVs
            // instead of the now-disabled backdrop path.
            #[cfg(target_os = "android")]
            let backdrop_source_y =
                (viewport_h as f32 - entry.source_rect.1 - entry.source_rect.3).max(0.0);
            #[cfg(not(target_os = "android"))]
            let backdrop_source_y = entry.source_rect.1;
            let (source_x, source_y, source_w, source_h) =
                if USE_FULL_FRAME_BACKDROP_BLUR && res.backdrop_view.is_some() {
                    (
                        entry.source_rect.0,
                        backdrop_source_y,
                        entry.source_rect.2,
                        entry.source_rect.3,
                    )
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                };
            let overlay_data: [f32; 12] = [
                overlay_color.components[0],
                overlay_color.components[1],
                overlay_color.components[2],
                overlay_color.components[3],
                entry.border_radius as f32,
                entry.width as f32,
                entry.height as f32,
                if entry.blur_style == 1 || entry.blur_style == 3 {
                    1.0
                } else {
                    0.0
                },
                source_x,
                source_y,
                source_w,
                source_h,
            ];

            if entry.composite_uniform_buffer.is_none() {
                entry.composite_uniform_buffer =
                    Some(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Blur Legacy Composite Uniform Buffer"),
                        size: 48,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
            }
            if entry.composite_overlay_buffer.is_none() {
                entry.composite_overlay_buffer =
                    Some(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Blur Legacy Composite Overlay Buffer"),
                        size: 48,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
            }
            let uniform_buffer = entry.composite_uniform_buffer.as_ref().unwrap();
            let overlay_buffer = entry.composite_overlay_buffer.as_ref().unwrap();
            if entry.last_composite_uniform_data != Some(uniform_data) {
                queue.write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&uniform_data));
                entry.last_composite_uniform_data = Some(uniform_data);
            }
            if entry.last_composite_overlay_data != Some(overlay_data) {
                queue.write_buffer(overlay_buffer, 0, bytemuck::cast_slice(&overlay_data));
                entry.last_composite_overlay_data = Some(overlay_data);
            }

            let uses_backdrop = USE_FULL_FRAME_BACKDROP_BLUR && res.backdrop_view.is_some();
            if entry.composite_bind_group.is_none()
                || entry.composite_uses_backdrop != uses_backdrop
            {
                let source_view = res.backdrop_view.unwrap_or(&entry.texture_view);
                entry.composite_bind_group =
                    Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(&format!(
                            "Blur Legacy Composite Bind Group {}",
                            entry.view_id
                        )),
                        layout: res.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(source_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(res.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: uniform_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: overlay_buffer.as_entire_binding(),
                            },
                        ],
                    }));
                entry.composite_uses_backdrop = uses_backdrop;
            }
            rp.set_bind_group(0, entry.composite_bind_group.as_ref().unwrap(), &[]);
            rp.draw(0..6, 0..1);
        }

        if !is_visible || entry.children_texture_view.is_none() {
            continue;
        }
        // === Draw per-entry children overlay ===
        if let Some(ref children_view) = entry.children_texture_view {
            let bx = entry.children_bounds.0 as f64;
            let by = entry.children_bounds.1 as f64;
            let bw = entry.children_bounds.2 as f64;
            let bh = entry.children_bounds.3 as f64;
            let children_transform = Affine::translate((bx, by));
            let cmat = children_transform.as_coeffs();
            let ctex_width = bw as f32;
            let ctex_height = bh as f32;
            let children_uniform_data: [f32; 12] = [
                cmat[0] as f32 * ctex_width * scale_x,
                cmat[2] as f32 * ctex_width * scale_x,
                0.0,
                0.0,
                cmat[1] as f32 * ctex_height * scale_y,
                cmat[3] as f32 * ctex_height * scale_y,
                0.0,
                0.0,
                cmat[4] as f32 * scale_x + offset_x,
                cmat[5] as f32 * scale_y + offset_y,
                1.0,
                0.0,
            ];
            let children_overlay_data: [f32; 12] = [
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                ctex_width,
                ctex_height,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ];
            if entry.children_uniform_buffer.is_none() {
                entry.children_uniform_buffer =
                    Some(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Blur Children Composite Uniform Buffer"),
                        size: 48,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
            }
            if entry.children_overlay_buffer.is_none() {
                entry.children_overlay_buffer =
                    Some(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Blur Children Composite Overlay Buffer"),
                        size: 48,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
            }
            let children_uniform_buffer = entry.children_uniform_buffer.as_ref().unwrap();
            let children_overlay_buffer = entry.children_overlay_buffer.as_ref().unwrap();
            if entry.last_children_uniform_data != Some(children_uniform_data) {
                queue.write_buffer(
                    children_uniform_buffer,
                    0,
                    bytemuck::cast_slice(&children_uniform_data),
                );
                entry.last_children_uniform_data = Some(children_uniform_data);
            }
            if entry.last_children_overlay_data != Some(children_overlay_data) {
                queue.write_buffer(
                    children_overlay_buffer,
                    0,
                    bytemuck::cast_slice(&children_overlay_data),
                );
                entry.last_children_overlay_data = Some(children_overlay_data);
            }
            if entry.children_bind_group.is_none() {
                entry.children_bind_group =
                    Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(&format!("Children Composite Bind Group {}", entry.view_id)),
                        layout: res.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(children_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(res.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: children_uniform_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: children_overlay_buffer.as_entire_binding(),
                            },
                        ],
                    }));
            }
            rp.set_bind_group(0, entry.children_bind_group.as_ref().unwrap(), &[]);
            rp.draw(0..6, 0..1);
            log::debug!("[Blur Pass 4] Drew children for view_id={}", entry.view_id);
        }
    }

    !blurred_textures.is_empty()
}
