// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pass3 deferred-children helpers for frosted glass blur nodes.

use super::dirty::blur_entry_visible;
use super::types::{BlurDirtyKind, BlurredTextureEntry};
use crate::color::neutral_to_peniko_color;
use crate::text::{draw_prepared_text, GlyphRunCacheEntry, GlyphRunCacheKey, GlyphRunCacheStats};
use dyxel_render_api::{SceneNode, SharedMutex};
use kurbo::{Affine, Vec2};
use std::collections::HashMap;
use vello::peniko::Color;
use vello::Scene;

#[inline]
fn node_screen_offset(node: &SceneNode) -> Vec2 {
    if node.position_x != 0.0 || node.position_y != 0.0 {
        Vec2::new(node.position_x as f64, node.position_y as f64)
    } else {
        Vec2::new(node.x, node.y)
    }
}

#[inline]
fn node_visual_size(node: &SceneNode) -> (f64, f64) {
    if let dyxel_render_api::NodeContent::Text(ref payload) = node.content {
        (
            node.width.max(payload.measured_width as f64),
            node.height.max(payload.measured_height as f64),
        )
    } else {
        (node.width, node.height)
    }
}

/// Compute the axis-aligned bounding box of a subtree in screen coordinates.
/// Returns None if the node does not exist.
#[inline]
pub(crate) fn compute_subtree_bounds(
    id: u32,
    nodes: &HashMap<u32, &SceneNode>,
    parent_pos: Vec2,
) -> Option<kurbo::Rect> {
    let node = nodes.get(&id).copied()?;
    let offset = node_screen_offset(node);
    let (width, height) = node_visual_size(node);

    let global_x = parent_pos.x + offset.x;
    let global_y = parent_pos.y + offset.y;
    let mut bounds = kurbo::Rect::from_origin_size((global_x, global_y), (width, height));

    let child_pos = Vec2::new(global_x, global_y);
    for &child_id in &node.children {
        if let Some(child_bounds) = compute_subtree_bounds(child_id, nodes, child_pos) {
            bounds = bounds.union(child_bounds);
        }
    }

    Some(bounds)
}

/// Render a deferred child subtree (for frosted glass effect).
/// This renders children of blur views on top of the blurred background.
#[inline]
pub(crate) fn render_deferred_child(
    id: u32,
    nodes: &HashMap<u32, &SceneNode>,
    scene: &mut Scene,
    parent_pos: Vec2,
    origin_offset: Vec2,
    scene_transform: Affine,
    glyph_run_cache: &SharedMutex<HashMap<GlyphRunCacheKey, GlyphRunCacheEntry>>,
    glyph_run_cache_stats: &SharedMutex<GlyphRunCacheStats>,
) {
    use kurbo::{Rect as KRect, RoundedRect};
    use vello::peniko::{BlendMode as PenikoBlendMode, Compose, Fill, Mix};

    if let Some(node) = nodes.get(&id).copied() {
        let offset = node_screen_offset(node);
        let (width, height) = node_visual_size(node);
        let global_pos = parent_pos + offset;

        let local_transform = scene_transform
            * Affine::translate((
                global_pos.x - origin_offset.x,
                global_pos.y - origin_offset.y,
            ));

        // Apply opacity using layer if needed.
        let needs_layer = node.opacity < 1.0;
        if needs_layer {
            let alpha = node.opacity.clamp(0.0, 1.0);
            let blend = PenikoBlendMode::new(Mix::Normal, Compose::SrcOver);
            let rect = KRect::from_origin_size((0.0, 0.0), (width, height));
            scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &rect);
        }

        // Draw the child.
        if let dyxel_render_api::NodeContent::Text(ref payload) = node.content {
            draw_prepared_text(
                scene,
                payload,
                local_transform,
                glyph_run_cache,
                glyph_run_cache_stats,
                1.0,
            );
        } else if let dyxel_render_api::NodeContent::Rect { color } = node.content {
            let rect = KRect::from_origin_size((0.0, 0.0), (width, height));
            let pcolor = neutral_to_peniko_color(color);
            if node.border_radius > 0.0 {
                let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                scene.fill(Fill::NonZero, local_transform, pcolor, None, &rounded);
            } else {
                scene.fill(Fill::NonZero, local_transform, pcolor, None, &rect);
            }
        }

        if needs_layer {
            scene.pop_layer();
        }

        for &child_id in &node.children {
            render_deferred_child(
                child_id,
                nodes,
                scene,
                global_pos,
                origin_offset,
                scene_transform,
                glyph_run_cache,
                glyph_run_cache_stats,
            );
        }
    }
}

/// Render deferred children for blur entries that need it (Pass 3).
///
/// Iterates blur entries, skips invisible/empty/cached ones, creates
/// per-entry children scenes, and renders them to local textures.
///
/// Returns the indices of entries that were successfully rendered (for
/// debug frame saving by the caller).
#[inline]
pub(crate) fn render_pass3_children(
    blurred_textures: &mut [BlurredTextureEntry],
    node_map: &HashMap<u32, &SceneNode>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    aa_config: vello::AaConfig,
    glyph_run_cache: &SharedMutex<HashMap<GlyphRunCacheKey, GlyphRunCacheEntry>>,
    glyph_run_cache_stats: &SharedMutex<GlyphRunCacheStats>,
    viewport_w: u32,
    viewport_h: u32,
) -> Vec<usize> {
    let mut rendered_indices = Vec::new();

    for (entry_idx, entry) in blurred_textures.iter_mut().enumerate() {
        if !blur_entry_visible(entry, viewport_w, viewport_h) {
            continue;
        }
        if entry.deferred_children.is_empty() {
            continue;
        }
        if entry.children_bounds.2 <= 0.0 || entry.children_bounds.3 <= 0.0 {
            continue;
        }
        // Skip Pass 3 when children haven't changed and we already have a cached texture.
        if entry.dirty_kind != BlurDirtyKind::ChildrenChanged && entry.children_texture.is_some() {
            continue;
        }

        let mut children_scene = Scene::new();

        let global_x = entry.source_rect.0 as f64;
        let global_y = entry.source_rect.1 as f64;
        let origin_offset = Vec2::new(
            entry.children_bounds.0 as f64,
            entry.children_bounds.1 as f64,
        );
        let cw = entry.children_bounds.2.ceil() as u32;
        let ch = entry.children_bounds.3.ceil() as u32;
        // Keep Pass3 children in local screen/Y-down coordinates.
        // Do not bake Android/platform flips into glyph transforms:
        // Pass4 composites the children texture with normal local UV
        // sampling, and the Android blur backdrop is corrected only by
        // the Pass2 source copy Y mirror.
        let children_scene_transform = Affine::IDENTITY;

        for &child_id in &entry.deferred_children {
            render_deferred_child(
                child_id,
                node_map,
                &mut children_scene,
                Vec2::new(global_x, global_y),
                origin_offset,
                children_scene_transform,
                glyph_run_cache,
                glyph_run_cache_stats,
            );
        }

        let needs_new_children_texture = entry
            .children_texture
            .as_ref()
            .map_or(true, |t| t.width() != cw || t.height() != ch);

        if needs_new_children_texture {
            log::debug!(
                "[Blur] Pass 3: Creating local children texture {}x{} for view_id={}",
                cw,
                ch,
                entry.view_id
            );
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Children Local Texture"),
                size: wgpu::Extent3d {
                    width: cw,
                    height: ch,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            entry.children_texture = Some(texture);
            entry.children_texture_view = Some(view);
            entry.children_bind_group = None;
            entry.last_children_uniform_data = None;
            entry.last_children_overlay_data = None;
        }

        if let Some(ref view) = entry.children_texture_view {
            if let Err(e) = renderer.render_to_texture(
                device,
                queue,
                &children_scene,
                view,
                &vello::RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: cw,
                    height: ch,
                    antialiasing_method: aa_config,
                },
            ) {
                log::warn!(
                    "[Blur] Failed to render children texture for view_id={}: {:?}",
                    entry.view_id,
                    e
                );
            } else {
                rendered_indices.push(entry_idx);
            }
        }
    }

    rendered_indices
}
