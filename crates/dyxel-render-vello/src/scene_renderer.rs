// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Scene traversal and node drawing for `VelloBackend`.
//!
//! This module owns recursive scene construction; higher-level frame pass
//! orchestration remains in `lib.rs` / `render_helpers.rs`.

use crate::blur::entry::render_with_blur;
use crate::blur::types::BlurredTextureEntry;
use crate::cache::CachedDraw;
use crate::color::{apply_opacity_to_color, neutral_to_peniko_color};
use crate::state::RasterCacheLookup;
use crate::VelloBackend;
use dyxel_render_api::{NodeContent, SceneNode};
use kurbo::{Affine, Rect as KRect, RoundedRect, Vec2};
use std::collections::HashMap;
use vello::peniko::{BlendMode as PenikoBlendMode, Compose, Fill, Mix};
use vello::wgpu;
use vello::Scene;

/// Shared inputs and mutable outputs for recursive scene traversal.
///
/// This keeps recursion-focused methods from taking a long list of cross-cutting
/// renderer/cache/blur arguments while preserving the existing traversal
/// algorithm.
struct TraversalContext<'a> {
    nodes: &'a HashMap<u32, &'a SceneNode>,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    renderer: &'a mut vello::Renderer,
    filter_pipeline: Option<&'a crate::filter_pipeline::FilterPipeline>,
    blurred_textures: &'a mut Vec<BlurredTextureEntry>,
    cached_lookup: RasterCacheLookup<'a>,
    cached_draws: &'a mut Vec<CachedDraw>,
    blur_scene_frame: u64,
}

#[derive(Clone, Copy)]
struct NodeGeometry {
    global_pos: Vec2,
    local_transform: Affine,
    width: f64,
    height: f64,
}

impl NodeGeometry {
    #[inline]
    fn from_node(node: &SceneNode, parent_pos: Vec2, transform: Affine) -> Self {
        let global_pos = node_screen_position(node, parent_pos);
        Self {
            global_pos,
            local_transform: transform * Affine::translate((global_pos.x, global_pos.y)),
            width: node.width as f64,
            height: node.height as f64,
        }
    }
}

#[derive(Clone, Copy)]
struct LayerDecision {
    has_blur: bool,
    needs_layer: bool,
    needs_layer_without_blur: bool,
    node_in_blur_subtree: bool,
    direct_opacity: f32,
}

impl LayerDecision {
    #[inline]
    fn from_node(node: &SceneNode, in_blur_subtree: bool) -> Self {
        let has_blur = node.blur.is_some();
        let has_children = !node.children.is_empty();
        // OPTIMIZATION: Leaf nodes with only opacity don't need a layer.
        // We can apply opacity directly to the fill color, avoiding costly
        // per-tile clip commands that blow up the PTCL buffer.
        let needs_layer_for_opacity = node.opacity < 1.0 && has_children;
        let needs_layer = needs_layer_for_opacity || node.clip_to_bounds || has_blur;

        // NOTE: When blur is enabled, we skip layer creation here because:
        // 1. The node's background should NOT be drawn to the main scene
        // 2. Blur effect handles opacity and compositing separately
        let needs_layer_without_blur = needs_layer && !has_blur;

        let direct_opacity = if needs_layer_without_blur {
            1.0
        } else {
            node.opacity
        };

        Self {
            has_blur,
            needs_layer,
            needs_layer_without_blur,
            node_in_blur_subtree: in_blur_subtree || has_blur,
            direct_opacity,
        }
    }
}

#[inline]
fn node_screen_position(node: &SceneNode, parent_pos: Vec2) -> Vec2 {
    if node.position_x != 0.0 || node.position_y != 0.0 {
        parent_pos + Vec2::new(node.position_x as f64, node.position_y as f64)
    } else {
        parent_pos + Vec2::new(node.x, node.y)
    }
}

#[inline]
fn log_blur_node(id: u32, node: &SceneNode, geometry: NodeGeometry, layer: LayerDecision) {
    log::debug!(
        "[Debug] Blur node id={} blur_radius={} opacity={}",
        id,
        node.blur.as_ref().map(|b| b.blur_radius).unwrap_or(0.0),
        node.opacity
    );
    log::debug!(
        "[Debug] Position: taffy=({:.1},{:.1}) global=({:.1},{:.1}) size={:.1}x{:.1}",
        node.x,
        node.y,
        geometry.global_pos.x,
        geometry.global_pos.y,
        geometry.width,
        geometry.height
    );
    log::debug!(
        "[Debug] BEFORE check: id={} needs_layer={} has_blur=true needs_layer_without_blur={}",
        id,
        layer.needs_layer,
        layer.needs_layer_without_blur
    );
}

#[inline]
fn push_layer_if_needed(
    scene: &mut Scene,
    node: &SceneNode,
    geometry: NodeGeometry,
    layer: LayerDecision,
) {
    if !layer.needs_layer_without_blur {
        return;
    }

    // Convert opacity to layer alpha
    let alpha = node.opacity.clamp(0.0, 1.0);

    // Default blend mode (Normal)
    let blend = PenikoBlendMode::new(Mix::Normal, Compose::SrcOver);

    // Use node's bounds for the layer shape to avoid full-screen clip bloat.
    // If clip_to_bounds is enabled, we clip exactly to the node bounds.
    // Otherwise we still use node bounds (not infinite rect) for performance.
    let clip_rect = KRect::from_origin_size((0.0, 0.0), (geometry.width, geometry.height));
    if node.border_radius > 0.0 {
        let rounded_clip = RoundedRect::from_rect(clip_rect, node.border_radius as f64);
        scene.push_layer(
            Fill::NonZero,
            blend,
            alpha,
            geometry.local_transform,
            &rounded_clip,
        );
    } else {
        scene.push_layer(
            Fill::NonZero,
            blend,
            alpha,
            geometry.local_transform,
            &clip_rect,
        );
    }
}

#[inline]
fn try_emit_cached_draw(
    id: u32,
    geometry: NodeGeometry,
    layer: LayerDecision,
    ctx: &mut TraversalContext<'_>,
) -> bool {
    if layer.node_in_blur_subtree {
        return false;
    }

    ctx.cached_lookup.try_emit_cached_draw(
        id,
        Affine::translate((geometry.global_pos.x, geometry.global_pos.y)),
        geometry.width as f32,
        geometry.height as f32,
        ctx.cached_draws,
    )
}

impl VelloBackend {
    /// Public entry point: borrows raster-cache lookup once, then delegates to
    /// the internal recursive renderer.
    pub(crate) fn render_node_recursive_with_transform(
        &self,
        id: u32,
        nodes: &HashMap<u32, &SceneNode>,
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
        self.raster_cache_state.with_cached_lookup(|cached_lookup| {
            let mut ctx = TraversalContext {
                nodes,
                device,
                queue,
                renderer,
                filter_pipeline,
                blurred_textures,
                cached_lookup,
                cached_draws,
                blur_scene_frame,
            };
            self.render_node_recursive_internal(
                id,
                scene,
                parent_pos,
                transform,
                in_blur_subtree,
                &mut ctx,
            );
        });
    }

    pub(crate) fn build_raster_cache_scene(
        &self,
        id: u32,
        nodes: &HashMap<u32, &SceneNode>,
        scene: &mut Scene,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut vello::Renderer,
        cached_lookup: RasterCacheLookup<'_>,
    ) {
        let mut bake_blurred = Vec::new();
        let mut cached_draws = Vec::new();
        let mut ctx = TraversalContext {
            nodes,
            device,
            queue,
            renderer,
            filter_pipeline: None,
            blurred_textures: &mut bake_blurred,
            cached_lookup,
            cached_draws: &mut cached_draws,
            blur_scene_frame: 0,
        };
        self.render_node_recursive_internal(
            id,
            scene,
            Vec2::ZERO,
            Affine::IDENTITY,
            false,
            &mut ctx,
        );
    }
}

// render_with_blur moved to blur/entry.rs
impl VelloBackend {
    #[inline]
    fn draw_shadow_if_needed(
        &self,
        id: u32,
        node: &SceneNode,
        scene: &mut Scene,
        geometry: NodeGeometry,
        layer: LayerDecision,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut vello::Renderer,
    ) {
        // === Step 1: Draw Shadow (if any, using blur) ===
        // Xilem pattern: Draw shadow first, then content on top
        // NOTE: When blur is enabled, skip shadow in Pass 1. Shadow will be handled
        // by the blur compositing pipeline to avoid double-rendering.
        log::debug!(
            "[ShadowCheck] id={} has_shadow={} has_blur={}",
            id,
            node.shadow.is_some(),
            layer.has_blur
        );
        if let Some(ref shadow) = node.shadow {
            if !layer.has_blur {
                self.shadow_cache_state.draw_node_shadow(
                    id,
                    shadow,
                    scene,
                    geometry.local_transform,
                    geometry.width,
                    geometry.height,
                    node.border_radius as f64,
                    device,
                    queue,
                    renderer,
                );
            }
        }
    }

    #[inline]
    fn apply_blur_if_needed(
        &self,
        id: u32,
        node: &SceneNode,
        scene: &mut Scene,
        geometry: NodeGeometry,
        layer: LayerDecision,
        ctx: &mut TraversalContext<'_>,
    ) -> bool {
        if let (Some(blur), Some(filter_pipeline)) =
            (node.blur.as_ref(), ctx.filter_pipeline.as_ref())
        {
            render_with_blur(
                blur,
                id,
                ctx.nodes,
                scene,
                geometry.local_transform,
                geometry.global_pos,
                ctx.device,
                ctx.queue,
                &mut *ctx.renderer,
                filter_pipeline,
                geometry.width,
                geometry.height,
                layer.needs_layer,
                ctx.blurred_textures,
                ctx.blur_scene_frame,
            )
        } else {
            false
        }
    }

    #[inline]
    fn draw_node_content(
        &self,
        id: u32,
        node: &SceneNode,
        scene: &mut Scene,
        geometry: NodeGeometry,
        layer: LayerDecision,
    ) {
        match &node.content {
            NodeContent::Text(payload) => {
                self.text_cache_state.draw_prepared_text(
                    scene,
                    payload,
                    geometry.local_transform,
                    layer.direct_opacity,
                );
            }
            NodeContent::Rect { color } => {
                let rect = KRect::from_origin_size((0.0, 0.0), (geometry.width, geometry.height));
                // Apply opacity directly only when no layer is handling it
                let effective_color = if layer.direct_opacity < 1.0 {
                    apply_opacity_to_color(*color, layer.direct_opacity)
                } else {
                    *color
                };
                let pcolor = neutral_to_peniko_color(effective_color);

                // Debug: Log fill operations for non-text nodes
                log::debug!(
                    "[DebugFill] id={} color={:?} size={}x{} transform={:?}",
                    id,
                    color,
                    geometry.width,
                    geometry.height,
                    geometry.local_transform
                );

                if node.border_radius > 0.0 {
                    let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                    scene.fill(
                        Fill::NonZero,
                        geometry.local_transform,
                        pcolor,
                        None,
                        &rounded,
                    );
                } else {
                    scene.fill(Fill::NonZero, geometry.local_transform, pcolor, None, &rect);
                }
            }
        }
    }

    #[inline]
    fn render_children(
        &self,
        node: &SceneNode,
        scene: &mut Scene,
        transform: Affine,
        geometry: NodeGeometry,
        layer: LayerDecision,
        blur_applied: bool,
        ctx: &mut TraversalContext<'_>,
    ) {
        // === Step 5: Recursively render children ===
        // For blur views: skip children in Pass 1, they will be rendered to
        // a separate texture in Pass 3 and composited on top of blur in blit pass.
        // For non-blur views: render children normally.
        // DEBUG: Log children traversal
        if !node.children.is_empty() {
            log::debug!(
                "[DebugChildren] id={} has {} children: {:?}",
                node.id,
                node.children.len(),
                node.children
            );
        }
        if !blur_applied {
            for &child_id in &node.children {
                self.render_node_recursive_internal(
                    child_id,
                    scene,
                    geometry.global_pos,
                    transform,
                    layer.node_in_blur_subtree,
                    ctx,
                );
            }
        }
    }

    /// Render a node with layer effects (alpha, blur, shadow, clip)
    /// Following Xilem's pattern: shadow -> content -> children
    fn render_node_recursive_internal(
        &self,
        id: u32,
        scene: &mut Scene,
        parent_pos: Vec2,
        transform: Affine,
        in_blur_subtree: bool,
        ctx: &mut TraversalContext<'_>,
    ) {
        if let Some(node) = ctx.nodes.get(&id).copied() {
            let geometry = NodeGeometry::from_node(node, parent_pos, transform);
            let layer = LayerDecision::from_node(node, in_blur_subtree);

            // === Raster Cache Check ===
            // Conservative eligibility: only nodes fully outside any blur subtree.
            // Backend performs read-only lookup; Runtime decides which nodes to bake.
            if try_emit_cached_draw(id, geometry, layer, ctx) {
                return;
            }

            // Debug: Log blur node info
            if layer.has_blur {
                log_blur_node(id, node, geometry, layer);
            }

            self.draw_shadow_if_needed(
                id,
                node,
                scene,
                geometry,
                layer,
                ctx.device,
                ctx.queue,
                &mut *ctx.renderer,
            );

            // === Step 2: Push Layer (if needed for alpha/blur/clip) ===
            // NOTE: When blur is enabled, we skip layer creation here because:
            // 1. The node's background should NOT be drawn to the main scene
            // 2. Blur effect handles opacity and compositing separately

            log::debug!(
                "[LayerCheck] id={} needs_layer={} clip_to_bounds={} opacity={} border_radius={}",
                id,
                layer.needs_layer_without_blur,
                node.clip_to_bounds,
                node.opacity,
                node.border_radius
            );
            push_layer_if_needed(scene, node, geometry, layer);

            // === Step 3: Handle Blur Effect ===
            // If blur is enabled, render to offscreen texture and apply blur
            let blur_applied = self.apply_blur_if_needed(id, node, scene, geometry, layer, ctx);

            // === Step 4: Draw Node Content ===
            // Skip normal drawing if blur was applied (blur texture will be drawn in blit pass)
            // Opacity is applied either by the layer (Step 2) or baked into content color here.
            // If a layer was pushed for opacity, we must NOT double-apply it to content.
            if !blur_applied {
                self.draw_node_content(id, node, scene, geometry, layer);
            }

            self.render_children(node, scene, transform, geometry, layer, blur_applied, ctx);

            // === Step 6: Pop Layer (if pushed) ===
            // Only pop layer if we pushed it (when blur is NOT enabled)
            if layer.needs_layer_without_blur {
                scene.pop_layer();
            }
        }
    }
}
