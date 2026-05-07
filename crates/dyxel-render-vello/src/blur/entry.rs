// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Blur entry lifecycle: creation, dirty detection, and texture allocation.

use super::children::compute_subtree_bounds;
use super::dirty::{
    blur_rect_changed, blur_texture_alloc_extent_px, kawase_pass_class_for_radius,
    quantize_blur_pos_px, quantize_blur_size_px, BLUR_SOURCE_RECT_EPS_PX, PARAM_DIRTY_RADIUS,
    PARAM_DIRTY_SRC_H, PARAM_DIRTY_SRC_W, PARAM_DIRTY_SRC_X, PARAM_DIRTY_SRC_Y, PARAM_DIRTY_STYLE,
};
use super::types::{BlurDirtyKind, BlurredTextureEntry};
use crate::color::neutral_to_peniko_color;
use dyxel_render_api::BlurEffect;
use kurbo::{Affine, Vec2};
use std::collections::HashMap;

/// Prepare or update a blur entry for the given node.
///
/// Called during scene building. Computes blur texture sizing, dirty detection,
/// and creates or updates the `BlurredTextureEntry`. Returns `true` if blur
/// was applied.
pub(crate) fn render_with_blur(
    blur: &BlurEffect,
    id: u32,
    nodes: &HashMap<u32, &dyxel_render_api::SceneNode>,
    _scene: &mut vello::Scene,
    local_transform: Affine,
    screen_origin: Vec2,
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    _renderer: &mut vello::Renderer,
    _filter_pipeline: &crate::filter_pipeline::FilterPipeline,
    node_width: f64,
    node_height: f64,
    _needs_layer: bool,
    blurred_textures: &mut Vec<BlurredTextureEntry>,
    blur_scene_frame: u64,
) -> bool {
    // Calculate padded size for blur (need extra space for blur bleed)
    let blur_radius = blur.blur_radius as f64;
    let padding = (blur_radius * 2.5).ceil() as u32;

    // ── LOD selection: sample from a lower-res backdrop pyramid level ──
    let backdrop_lod: u8 = if blur_radius <= 4.0 {
        1 // half-res
    } else if blur_radius <= 12.0 {
        2 // quarter-res
    } else {
        3 // eighth-res
    };
    // Keep blur texture at full size — only the source backdrop is downsampled.
    // This avoids transform/padding mismatch between render_with_blur and Pass 2.
    let content_width_px = quantize_blur_size_px(node_width as f32).max(1.0) as u32;
    let content_height_px = quantize_blur_size_px(node_height as f32).max(1.0) as u32;
    let texture_width = (content_width_px + padding * 2).max(1);
    let texture_height = (content_height_px + padding * 2).max(1);

    // Check if we already have an entry for this view_id (caching)
    let existing_index = blurred_textures.iter().position(|e| e.view_id == id);
    let bucket_alloc_width = blur_texture_alloc_extent_px(texture_width);
    let bucket_alloc_height = blur_texture_alloc_extent_px(texture_height);
    let (allocated_texture_width, allocated_texture_height) =
        existing_index.map_or((bucket_alloc_width, bucket_alloc_height), |idx| {
            let entry = &blurred_textures[idx];
            (
                entry.allocated_width.max(bucket_alloc_width),
                entry.allocated_height.max(bucket_alloc_height),
            )
        });
    let needs_new_texture = existing_index.map_or(true, |idx| {
        let entry = &blurred_textures[idx];
        entry.allocated_width < texture_width || entry.allocated_height < texture_height
    });

    let offscreen_texture = if needs_new_texture {
        let texture_desc = wgpu::TextureDescriptor {
            label: Some("Blur Offscreen Texture"),
            size: wgpu::Extent3d {
                width: allocated_texture_width,
                height: allocated_texture_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let tex = device.create_texture(&texture_desc);
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        Some((tex, view))
    } else {
        None
    };

    // Blur compositing happens in our own wgpu pass, whose coordinates are
    // screen/Y-down pixels. On Android the Vello scene itself is rendered with
    // `platform_correction`, so `local_transform` contains a Y flip. Do not use
    // that transform for blur source/composite metadata or the offscreen blur
    // patch will be vertically mirrored. Keep the corrected Vello transform only
    // for debug/logging; use the uncorrected screen origin for Pass 2/4.
    let screen_transform = Affine::translate((screen_origin.x, screen_origin.y));

    // Store the blurred texture for compositing in the final blit pass.
    // Adjust transform to account for the padding offset.
    let final_transform =
        screen_transform * Affine::translate((-(padding as f64), -(padding as f64)));

    // Calculate source rectangle in screen/texture coordinates for two-pass
    // rendering. The Pass 1 scene texture is blitted with normal Y-down UVs, so
    // blur copy should use the same screen Y on every platform.
    let source_x = quantize_blur_pos_px(screen_origin.x as f32);
    let source_y_taffy = quantize_blur_pos_px(screen_origin.y as f32);

    // Collect deferred children - they will be rendered after the blurred background
    let deferred_children: Vec<u32> = blur.deferred_children.clone();

    // Compute bounding box of deferred children for local rendering
    let mut children_bounds_rect: Option<kurbo::Rect> = None;
    for &child_id in &deferred_children {
        if let Some(bounds) = compute_subtree_bounds(
            child_id,
            nodes,
            Vec2::new(source_x as f64, source_y_taffy as f64),
        ) {
            children_bounds_rect = Some(children_bounds_rect.map_or(bounds, |r| r.union(bounds)));
        }
    }
    let padding_px = 6.0f64;
    let children_bounds = children_bounds_rect.map_or((0.0f32, 0.0f32, 0.0f32, 0.0f32), |r| {
        let x0 = (r.x0 - padding_px).floor().max(0.0) as f32;
        let y0 = (r.y0 - padding_px).floor().max(0.0) as f32;
        let x1 = (r.x1 + padding_px).ceil() as f32;
        let y1 = (r.y1 + padding_px).ceil() as f32;
        (x0, y0, x1 - x0, y1 - y0)
    });

    log::debug!(
        "[Blur] view_id={} source_rect=({:.1},{:.1}) size={:.1}x{:.1} parent_bg_check: y={:.1} h={:.1}",
        id,
        source_x,
        source_y_taffy,
        node_width,
        node_height,
        local_transform.as_coeffs()[5] - node_height,
        node_height
    );

    if let Some(index) = existing_index {
        // Update existing entry's metadata but reuse the texture
        let entry = &mut blurred_textures[index];
        entry.last_seen_frame = blur_scene_frame;

        // ── Per-entry dirty detection ──
        let radius_value_changed = (entry.prev_blur_radius - blur.blur_radius).abs() > 0.25;
        let style_changed = blur.blur_style != entry.blur_style;
        let blur_kernel_changed = radius_value_changed
            && kawase_pass_class_for_radius(entry.prev_blur_radius)
                != kawase_pass_class_for_radius(blur.blur_radius);
        let new_source_rect = (
            source_x,
            source_y_taffy,
            quantize_blur_size_px(node_width as f32),
            quantize_blur_size_px(node_height as f32),
        );
        let rect_changed = blur_rect_changed(entry.prev_source_rect, new_source_rect);
        let mut param_dirty_bits = 0u32;
        if blur_kernel_changed {
            param_dirty_bits |= PARAM_DIRTY_RADIUS;
        }
        if style_changed {
            param_dirty_bits |= PARAM_DIRTY_STYLE;
        }
        if (entry.prev_source_rect.0 - new_source_rect.0).abs() >= BLUR_SOURCE_RECT_EPS_PX {
            param_dirty_bits |= PARAM_DIRTY_SRC_X;
        }
        if (entry.prev_source_rect.1 - new_source_rect.1).abs() >= BLUR_SOURCE_RECT_EPS_PX {
            param_dirty_bits |= PARAM_DIRTY_SRC_Y;
        }
        if (entry.prev_source_rect.2 - new_source_rect.2).abs() >= BLUR_SOURCE_RECT_EPS_PX {
            param_dirty_bits |= PARAM_DIRTY_SRC_W;
        }
        if (entry.prev_source_rect.3 - new_source_rect.3).abs() >= BLUR_SOURCE_RECT_EPS_PX {
            param_dirty_bits |= PARAM_DIRTY_SRC_H;
        }
        let active_size_changed = entry.width != texture_width || entry.height != texture_height;
        let allocation_changed =
            entry.allocated_width < texture_width || entry.allocated_height < texture_height;
        let opacity_changed = (entry.prev_opacity - blur.opacity).abs() > f32::EPSILON;
        let overlay_changed = entry.prev_overlay_color != blur.overlay_color;
        let children_bounds_changed = (entry.children_bounds.0 - children_bounds.0).abs()
            >= BLUR_SOURCE_RECT_EPS_PX
            || (entry.children_bounds.1 - children_bounds.1).abs() >= BLUR_SOURCE_RECT_EPS_PX
            || (entry.children_bounds.2 - children_bounds.2).abs() >= BLUR_SOURCE_RECT_EPS_PX
            || (entry.children_bounds.3 - children_bounds.3).abs() >= BLUR_SOURCE_RECT_EPS_PX;
        let children_size_changed = (entry.children_bounds.2 - children_bounds.2).abs()
            >= BLUR_SOURCE_RECT_EPS_PX
            || (entry.children_bounds.3 - children_bounds.3).abs() >= BLUR_SOURCE_RECT_EPS_PX;
        let children_changed =
            entry.deferred_children != deferred_children || children_bounds_changed;

        let computed_dirty_kind = if allocation_changed {
            BlurDirtyKind::BackgroundChanged
        } else if blur_kernel_changed || rect_changed || active_size_changed {
            BlurDirtyKind::BlurParamsChanged
        } else if opacity_changed || overlay_changed || style_changed {
            BlurDirtyKind::OverlayOnlyChanged
        } else if children_changed {
            BlurDirtyKind::ChildrenChanged
        } else {
            BlurDirtyKind::Clean
        };
        entry.dirty_kind = if entry.blur_rebuild_pending
            && matches!(
                computed_dirty_kind,
                BlurDirtyKind::Clean | BlurDirtyKind::OverlayOnlyChanged
            ) {
            BlurDirtyKind::BlurParamsChanged
        } else {
            computed_dirty_kind
        };
        entry.param_dirty_bits = if entry.dirty_kind == BlurDirtyKind::BlurParamsChanged {
            param_dirty_bits
        } else {
            0
        };
        if matches!(
            entry.dirty_kind,
            BlurDirtyKind::BackgroundChanged | BlurDirtyKind::BlurParamsChanged
        ) {
            entry.blur_rebuild_pending = true;
        }

        // Update prev_* snapshots for next frame's comparison
        entry.prev_blur_radius = blur.blur_radius;
        entry.prev_source_rect = new_source_rect;
        entry.prev_opacity = blur.opacity;
        entry.prev_overlay_color = blur.overlay_color;

        entry.transform = final_transform;
        entry.opacity = blur.opacity;
        entry.overlay_color = neutral_to_peniko_color(blur.overlay_color);
        entry.border_radius = blur.border_radius as f64;
        entry.source_rect = new_source_rect;
        entry.deferred_children = deferred_children;
        entry.children_bounds = children_bounds;
        entry.backdrop_lod = backdrop_lod;
        if active_size_changed {
            entry.width = texture_width;
            entry.height = texture_height;
            entry.atlas_valid = false;
            entry.atlas_dirty = true;
        }
        if children_size_changed {
            entry.children_texture = None;
            entry.children_texture_view = None;
            entry.children_bind_group = None;
            entry.last_children_uniform_data = None;
            entry.last_children_overlay_data = None;
        }
        entry.blur_radius = blur.blur_radius;
        entry.blur_style = blur.blur_style;
        entry.skipped_due_to_size = false;
        if allocation_changed {
            log::debug!(
                "[Blur] Recreating texture for view_id={} due to allocation growth (active {}x{} -> {}x{}, alloc {}x{} -> {}x{})",
                id,
                entry.width,
                entry.height,
                texture_width,
                texture_height,
                entry.allocated_width,
                entry.allocated_height,
                allocated_texture_width,
                allocated_texture_height
            );
            let (tex, view) =
                offscreen_texture.expect("allocation_changed implies needs_new_texture");
            entry.texture = tex;
            entry.texture_view = view;
            entry.width = texture_width;
            entry.height = texture_height;
            entry.allocated_width = allocated_texture_width;
            entry.allocated_height = allocated_texture_height;
            entry.blur_valid = false;
            entry.blur_rebuild_pending = true;
            entry.atlas_valid = false;
            entry.atlas_dirty = true;
            entry.composite_bind_group = None;
            entry.last_composite_uniform_data = None;
            entry.last_composite_overlay_data = None;
            entry.composite_uses_backdrop = false;
        } else {
            log::debug!(
                "[Blur] Reusing cached texture for view_id={} dirty={:?}",
                id,
                entry.dirty_kind
            );
        }
    } else {
        // Create new entry
        let (tex, view) = offscreen_texture.expect("new entry must have texture");
        let new_source_rect = (
            source_x,
            source_y_taffy,
            quantize_blur_size_px(node_width as f32),
            quantize_blur_size_px(node_height as f32),
        );
        blurred_textures.push(BlurredTextureEntry {
            texture: tex,
            texture_view: view,
            width: texture_width,
            height: texture_height,
            allocated_width: allocated_texture_width,
            allocated_height: allocated_texture_height,
            transform: final_transform,
            opacity: blur.opacity,
            overlay_color: neutral_to_peniko_color(blur.overlay_color),
            border_radius: blur.border_radius as f64,
            source_rect: new_source_rect,
            deferred_children,
            children_bounds,
            children_texture: None,
            children_texture_view: None,
            view_id: id,
            blur_radius: blur.blur_radius,
            blur_style: blur.blur_style,
            skipped_due_to_size: false,
            dirty_kind: BlurDirtyKind::BackgroundChanged,
            prev_blur_radius: blur.blur_radius,
            prev_source_rect: new_source_rect,
            prev_opacity: blur.opacity,
            prev_overlay_color: blur.overlay_color,
            param_dirty_bits: 0,
            backdrop_lod,
            last_seen_frame: blur_scene_frame,
            blur_valid: false,
            blur_rebuild_pending: true,
            last_blur_rebuild_frame: 0,
            composite_uniform_buffer: None,
            composite_overlay_buffer: None,
            composite_bind_group: None,
            composite_uses_backdrop: false,
            last_composite_uniform_data: None,
            last_composite_overlay_data: None,
            children_uniform_buffer: None,
            children_overlay_buffer: None,
            children_bind_group: None,
            last_children_uniform_data: None,
            last_children_overlay_data: None,
            atlas_valid: false,
            atlas_dirty: true,
            atlas_x: 0,
            atlas_y: 0,
        });
    }

    true
}
