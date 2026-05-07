// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shadow cache data structures for the Vello backend.

use crate::color::neutral_to_peniko_color;
use crate::FRAME_COUNTER;
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use dyxel_render_api::{ShadowDesc, SharedMutex};
use kurbo::{Affine, Rect as KRect};
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use vello::peniko;
use vello::wgpu;
use vello::Scene;

/// Key for caching pre-rendered shadow textures.
/// Shadows with identical geometry + style can share the same cached texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ShadowCacheKey {
    /// Node width in pixels.
    pub(crate) width: u16,
    /// Node height in pixels.
    pub(crate) height: u16,
    /// Border radius (quantized to 0.5px, stored as x2).
    pub(crate) border_radius: u16,
    /// Shadow blur radius (quantized to 0.5px, stored as x2).
    pub(crate) blur_radius: u16,
    /// Shadow color (RGBA).
    pub(crate) color: [u8; 4],
}

pub(crate) struct ShadowCacheEntry {
    /// Vello ImageData registered with the renderer.
    /// The renderer internally holds the wgpu::Texture through image_overrides.
    pub(crate) image_data: peniko::ImageData,
    /// Last frame this entry was used (for LRU eviction).
    pub(crate) last_used_frame: AtomicU64,
}

#[derive(Default, Debug)]
pub(crate) struct ShadowCacheStats {
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) evictions: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct ShadowCacheRefs<'a> {
    pub(crate) cache: &'a SharedMutex<HashMap<ShadowCacheKey, ShadowCacheEntry>>,
    pub(crate) stats: &'a SharedMutex<ShadowCacheStats>,
    pub(crate) misses_this_frame: &'a AtomicU64,
}

/// Draw one node shadow using the shared shadow cache.
///
/// This is intentionally still a small orchestration helper: scene traversal,
/// blur/layer decisions, and child recursion remain in `lib.rs`.
pub(crate) fn draw_node_shadow(
    id: u32,
    shadow: &ShadowDesc,
    scene: &mut Scene,
    local_transform: Affine,
    node_width: f64,
    node_height: f64,
    border_radius: f64,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    cache_refs: ShadowCacheRefs<'_>,
) {
    log::debug!(
        "[ShadowDraw] id={} offset=({},{}) blur={} color={:?}",
        id,
        shadow.offset_x,
        shadow.offset_y,
        shadow.blur,
        shadow.color
    );
    let shadow_x = shadow.offset_x as f64;
    let shadow_y = shadow.offset_y as f64;
    let blur_radius = shadow.blur as f64;
    let shadow_color = neutral_to_peniko_color(shadow.color);

    // Try shadow cache first.
    let cache_key = ShadowCacheKey {
        width: node_width as u16,
        height: node_height as u16,
        border_radius: (border_radius * 2.0) as u16,
        blur_radius: (shadow.blur * 2.0) as u16,
        color: shadow.color,
    };

    let mut cache_guard = cache_refs.cache.lock().unwrap();
    if let Some(entry) = cache_guard.get_mut(&cache_key) {
        // Cache hit: draw cached shadow texture.
        entry
            .last_used_frame
            .store(FRAME_COUNTER.load(Ordering::Relaxed), Ordering::Relaxed);
        let brush = peniko::ImageBrush::new(entry.image_data.clone())
            .with_alpha(shadow_color.components[3]);
        let blur_pad = blur_radius * 2.0;
        let image_transform =
            local_transform * Affine::translate((shadow_x - blur_pad, shadow_y - blur_pad));
        scene.draw_image(&brush, image_transform);
        drop(cache_guard);
        cache_refs.stats.lock().unwrap().hits += 1;
        return;
    }
    drop(cache_guard);
    cache_refs.stats.lock().unwrap().misses += 1;

    // Cap per-frame cache misses to avoid GPU submit spikes.
    // On cold start shadows warm up over ~30 frames instead of one.
    let misses = cache_refs.misses_this_frame.fetch_add(1, Ordering::Relaxed);
    if misses >= 5 {
        draw_direct_shadow(
            scene,
            local_transform,
            shadow_x,
            shadow_y,
            node_width,
            node_height,
            border_radius,
            blur_radius,
            shadow_color,
        );
        return;
    }

    // Cache miss: render shadow to a new texture and cache it.
    let blur_pad = blur_radius * 2.0;
    let tex_w = (node_width + blur_pad * 2.0).ceil() as u32;
    let tex_h = (node_height + blur_pad * 2.0).ceil() as u32;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow_cache_texture"),
        size: wgpu::Extent3d {
            width: tex_w.max(1),
            height: tex_h.max(1),
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

    let mut shadow_scene = Scene::new();
    let rect = KRect::from_origin_size((blur_pad, blur_pad), (node_width, node_height));
    if border_radius > 0.0 {
        shadow_scene.draw_blurred_rounded_rect(
            Affine::IDENTITY,
            rect,
            shadow_color,
            border_radius,
            blur_radius,
        );
    } else {
        shadow_scene.draw_blurred_rounded_rect(
            Affine::IDENTITY,
            rect,
            shadow_color,
            0.0,
            blur_radius,
        );
    }

    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    match renderer.render_to_texture(
        device,
        queue,
        &shadow_scene,
        &texture_view,
        &vello::RenderParams {
            base_color: peniko::Color::new([0.0, 0.0, 0.0, 0.0]),
            width: tex_w,
            height: tex_h,
            antialiasing_method: vello::AaConfig::Area,
        },
    ) {
        Ok(()) => {
            let image_data = renderer.register_texture(texture);

            // Draw the newly cached shadow in the current frame.
            let brush =
                peniko::ImageBrush::new(image_data.clone()).with_alpha(shadow_color.components[3]);
            let image_transform =
                local_transform * Affine::translate((shadow_x - blur_pad, shadow_y - blur_pad));
            scene.draw_image(&brush, image_transform);

            let entry = ShadowCacheEntry {
                image_data,
                last_used_frame: AtomicU64::new(FRAME_COUNTER.load(Ordering::Relaxed)),
            };
            cache_refs.cache.lock().unwrap().insert(cache_key, entry);
        }
        Err(e) => {
            log::warn!(
                "[ShadowCache] render_to_texture failed: {:?}. Falling back to direct draw.",
                e
            );
            draw_direct_shadow(
                scene,
                local_transform,
                shadow_x,
                shadow_y,
                node_width,
                node_height,
                border_radius,
                blur_radius,
                shadow_color,
            );
        }
    }
}

fn draw_direct_shadow(
    scene: &mut Scene,
    local_transform: Affine,
    shadow_x: f64,
    shadow_y: f64,
    node_width: f64,
    node_height: f64,
    border_radius: f64,
    blur_radius: f64,
    shadow_color: peniko::Color,
) {
    let shadow_rect = KRect::from_origin_size((shadow_x, shadow_y), (node_width, node_height));
    scene.draw_blurred_rounded_rect(
        local_transform,
        shadow_rect,
        shadow_color,
        border_radius,
        blur_radius,
    );
}
