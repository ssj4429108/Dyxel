// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pass 3.5: Pack blurred textures into an atlas for instanced composite.

use super::types::{
    BlurAtlasLayout, BlurAtlasTexture, BlurFrameUniform, BlurInstance, BlurredTextureEntry,
};

/// Result of the atlas pack pass.
pub(crate) struct AtlasPackResult {
    pub(crate) bind_group: Option<wgpu::BindGroup>,
    pub(crate) instance_count: u32,
    pub(crate) enabled: bool,
}

/// Pack blurred textures into an atlas for instanced composite.
///
/// Copies per-entry blur textures into atlas slots, creates GPU instance data,
/// writes frame uniforms and instance buffers, and builds the instanced bind group.
///
/// The caller must have already called `ensure_blur_instanced_resources` and
/// `ensure_blur_atlas_texture` before invoking this function.
#[inline]
pub(crate) fn pack_blur_atlas(
    blurred_textures: &mut [BlurredTextureEntry],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    post_enc: &mut wgpu::CommandEncoder,
    atlas: &BlurAtlasTexture,
    atlas_wide_blur_valid: bool,
    layout: &BlurAtlasLayout,
    frame_uniform_buffer: &wgpu::Buffer,
    instance_buffer: &wgpu::Buffer,
    instanced_bind_group_layout: &wgpu::BindGroupLayout,
    cached_bind_group: &mut Option<wgpu::BindGroup>,
    sampler: &wgpu::Sampler,
    viewport_w: u32,
    viewport_h: u32,
    atlas_wide_source_copies: usize,
    diag_log_every_n_frames: u64,
    frame_counter: u64,
) -> AtlasPackResult {
    let mut instances: Vec<BlurInstance> = Vec::with_capacity(layout.placements.len());
    let mut atlas_copies = 0usize;

    for &(idx, ax, ay) in &layout.placements {
        let entry = &mut blurred_textures[idx];
        let placement_changed = !entry.atlas_valid || entry.atlas_x != ax || entry.atlas_y != ay;
        if placement_changed {
            entry.atlas_x = ax;
            entry.atlas_y = ay;
            entry.atlas_valid = atlas_wide_blur_valid;
            entry.atlas_dirty = !atlas_wide_blur_valid;
        }
        if !entry.blur_valid {
            continue;
        }

        if atlas_wide_blur_valid {
            entry.atlas_x = ax;
            entry.atlas_y = ay;
            entry.atlas_valid = true;
            entry.atlas_dirty = false;
        } else if entry.atlas_dirty || !entry.atlas_valid {
            post_enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &entry.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &atlas.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: ax, y: ay, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: entry.width,
                    height: entry.height,
                    depth_or_array_layers: 1,
                },
            );
            entry.atlas_dirty = false;
            entry.atlas_valid = true;
            atlas_copies += 1;
        }

        let mat = entry.transform.as_coeffs();
        let overlay_color = entry.overlay_color;
        instances.push(BlurInstance {
            rect: [
                mat[4] as f32,
                mat[5] as f32,
                entry.width as f32,
                entry.height as f32,
            ],
            source_rect: [
                entry.atlas_x as f32,
                entry.atlas_y as f32,
                entry.width as f32,
                entry.height as f32,
            ],
            color: [
                overlay_color.components[0],
                overlay_color.components[1],
                overlay_color.components[2],
                overlay_color.components[3],
            ],
            params: [
                entry.border_radius as f32,
                entry.opacity,
                if entry.blur_style == 1 || entry.blur_style == 3 {
                    1.0
                } else {
                    0.0
                },
                0.0,
            ],
        });
    }

    let frame = BlurFrameUniform {
        viewport_size: [viewport_w as f32, viewport_h as f32],
        _pad: [0.0, 0.0],
    };
    queue.write_buffer(frame_uniform_buffer, 0, bytemuck::bytes_of(&frame));
    queue.write_buffer(instance_buffer, 0, bytemuck::cast_slice(&instances));

    if cached_bind_group.is_none() {
        *cached_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blur Atlas Instanced Bind Group"),
            layout: instanced_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: frame_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: instance_buffer.as_entire_binding(),
                },
            ],
        }));
    }

    let instance_count = instances.len() as u32;
    let enabled = instance_count > 0;

    if enabled && frame_counter % diag_log_every_n_frames == 0 {
        log::info!(
            "[BlurAtlas] compositing {} {} blur entries via atlas {}x{} slot={} gap={}, copies={}",
            instance_count,
            if atlas_wide_blur_valid {
                "atlas-wide"
            } else {
                "legacy"
            },
            layout.width,
            layout.height,
            layout.slot,
            layout.gap,
            if atlas_wide_blur_valid {
                atlas_wide_source_copies
            } else {
                atlas_copies
            }
        );
    }

    AtlasPackResult {
        bind_group: cached_bind_group.clone(),
        instance_count,
        enabled,
    }
}
