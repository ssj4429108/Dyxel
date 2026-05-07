// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Legacy per-entry blur passes: index selection, source copy, and Kawase blur.

use super::dirty::blur_entry_visible;
#[cfg(not(target_os = "android"))]
use super::dirty::MAX_BLUR_REBUILDS_PER_FRAME;
#[cfg(target_os = "android")]
use super::dirty::MAX_BLUR_REBUILDS_PER_FRAME_AT_60HZ;
use super::types::{BlurDirtyKind, BlurredTextureEntry};
use crate::filter_pipeline::FilterPipeline;
use crate::texture_pool::SharedTexturePool;

/// Select which blur entries need a full rebuild this frame.
///
/// Filters by visibility and dirty state, sorts by priority (invalid first,
/// then oldest rebuild, then largest area), and truncates to the per-platform
/// rebuild budget.
#[inline]
pub(crate) fn select_legacy_rebuild_indices(
    blurred_textures: &[BlurredTextureEntry],
    viewport_w: u32,
    viewport_h: u32,
) -> Vec<usize> {
    let mut rebuild_indices: Vec<usize> = blurred_textures
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            !entry.skipped_due_to_size
                && blur_entry_visible(entry, viewport_w, viewport_h)
                && (!entry.blur_valid
                    || entry.blur_rebuild_pending
                    || matches!(
                        entry.dirty_kind,
                        BlurDirtyKind::BackgroundChanged | BlurDirtyKind::BlurParamsChanged
                    ))
        })
        .map(|(idx, _)| idx)
        .collect();

    rebuild_indices.sort_by_key(|&idx| {
        let entry = &blurred_textures[idx];
        (
            entry.blur_valid,
            entry.last_blur_rebuild_frame,
            std::cmp::Reverse((entry.width as u64) * (entry.height as u64)),
        )
    });

    // Keep rebuild pressure bounded. When the cadence governor is
    // targeting 60Hz, prioritize frame stability over catching up
    // stale blur entries quickly; cached blur remains visually
    // acceptable and invalid entries are filled gradually.
    #[cfg(target_os = "android")]
    let rebuild_budget = MAX_BLUR_REBUILDS_PER_FRAME_AT_60HZ;
    #[cfg(not(target_os = "android"))]
    let rebuild_budget = MAX_BLUR_REBUILDS_PER_FRAME;
    if rebuild_indices.len() > rebuild_budget {
        rebuild_indices.truncate(rebuild_budget);
    }

    rebuild_indices
}

/// Copy scene regions into per-entry blur source textures.
///
/// Contains the Android Y-mirror invariant: Vulkan-style inverted Y requires
/// mirroring the raw copy source on Android; composite shaders continue to
/// sample in normal local UVs.
///
/// Returns a list of `(idx, view_id, texture, blur_radius)` for entries that
/// need Kawase blur applied.
#[inline]
pub(crate) fn copy_legacy_blur_sources(
    blurred_textures: &mut [BlurredTextureEntry],
    post_enc: &mut wgpu::CommandEncoder,
    scene_texture: &wgpu::Texture,
    rebuild_indices: &[usize],
    viewport_w: u32,
    viewport_h: u32,
) -> Vec<(usize, u32, wgpu::Texture, f32)> {
    let mut blur_entries: Vec<(usize, u32, wgpu::Texture, f32)> = Vec::new();

    for &idx in rebuild_indices {
        let entry = &mut blurred_textures[idx];
        if entry.skipped_due_to_size {
            continue;
        }

        let (src_x, src_y, src_w, src_h) = entry.source_rect;
        let padding = ((entry.width as f32 - src_w) * 0.5).max(0.0) as u32;

        #[cfg(target_os = "android")]
        // Android/Vello writes the Pass 1 scene texture with a
        // Vulkan-style inverted Y relative to our screen-space
        // blur rects. Mirror only the raw copy source here; the
        // composite shaders continue to sample in normal local UVs.
        let src_origin_y = (viewport_h as f32 - src_y - src_h).max(0.0) as u32;
        #[cfg(not(target_os = "android"))]
        let src_origin_y = src_y.max(0.0) as u32;

        let src_origin_x = src_x.max(0.0) as u32;
        let copy_width = (src_w as u32)
            .min(viewport_w.saturating_sub(src_origin_x))
            .min(entry.width.saturating_sub(padding));
        let copy_height = (src_h as u32)
            .min(viewport_h.saturating_sub(src_origin_y))
            .min(entry.height.saturating_sub(padding));
        if copy_width == 0 || copy_height == 0 {
            continue;
        }

        post_enc.clear_texture(
            &entry.texture,
            &wgpu::ImageSubresourceRange {
                aspect: wgpu::TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: None,
                base_array_layer: 0,
                array_layer_count: None,
            },
        );
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
                texture: &entry.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: padding,
                    y: padding,
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

        if entry.blur_radius > 0.0 {
            blur_entries.push((idx, entry.view_id, entry.texture.clone(), entry.blur_radius));
        }
    }

    blur_entries
}

/// Apply Kawase blur to each entry that was successfully copied.
///
/// On success, marks entries as valid and cleans up dirty state.
#[inline]
pub(crate) fn apply_legacy_kawase_blur(
    blurred_textures: &mut [BlurredTextureEntry],
    pipeline: &FilterPipeline,
    post_enc: &mut wgpu::CommandEncoder,
    blur_entries: Vec<(usize, u32, wgpu::Texture, f32)>,
    texture_pool: Option<&SharedTexturePool>,
    current_frame: u64,
) {
    let mut rebuilt_indices = Vec::new();
    for (idx, view_id, texture, blur_radius) in blur_entries {
        let result = pipeline.apply_frosted_glass_kawase(
            post_enc,
            &texture,
            &texture,
            blur_radius,
            texture_pool,
        );
        if let Err(e) = result {
            log::warn!(
                "[BlurLegacy] Kawase failed for view_id={}: {:?}",
                view_id,
                e
            );
        } else {
            rebuilt_indices.push(idx);
        }
    }

    for idx in rebuilt_indices {
        if let Some(entry) = blurred_textures.get_mut(idx) {
            entry.blur_valid = true;
            entry.blur_rebuild_pending = false;
            entry.atlas_dirty = true;
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
}
