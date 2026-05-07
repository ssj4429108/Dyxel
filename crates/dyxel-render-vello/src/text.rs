// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Prepared text drawing and glyph-run caching for the Vello backend.

use crate::color::{apply_opacity_to_color, neutral_to_peniko_color};
use crate::FRAME_COUNTER;
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use dyxel_render_api::SharedMutex;
use kurbo::Affine;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use vello::{peniko, Scene};

/// Key for caching pre-built glyph runs.
/// Avoids re-iterating and re-mapping glyphs every frame for static text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GlyphRunCacheKey {
    pub(crate) font_ptr: usize,
    pub(crate) font_size_quanted: u32,
    pub(crate) color: [u8; 4],
    pub(crate) glyph_signature: u64,
}

pub(crate) struct GlyphRunCacheEntry {
    pub(crate) glyphs: Vec<vello::Glyph>,
    pub(crate) last_used_frame: AtomicU64,
}

#[derive(Default, Debug)]
pub(crate) struct GlyphRunCacheStats {
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) evictions: u64,
}

/// Draw prepared text payload into the scene.
/// Consumes PreparedText directly: decorations (selection/cursor) + glyph runs.
/// Uses GlyphRunCache to avoid re-mapping glyphs every frame for static text.
pub(crate) fn draw_prepared_text(
    scene: &mut Scene,
    payload: &dyxel_render_api::TextDrawPayload,
    transform: Affine,
    glyph_run_cache: &SharedMutex<HashMap<GlyphRunCacheKey, GlyphRunCacheEntry>>,
    glyph_run_cache_stats: &SharedMutex<GlyphRunCacheStats>,
    opacity: f32,
) {
    use peniko::{Brush, Fill};
    use std::hash::{Hash, Hasher};

    // 1. Draw decorations (selection background, cursor)
    for deco in &payload.prepared.decorations {
        let rect = kurbo::Rect::new(
            deco.x as f64,
            deco.y as f64,
            (deco.x + deco.width) as f64,
            (deco.y + deco.height) as f64,
        );
        scene.fill(
            Fill::NonZero,
            transform,
            neutral_to_peniko_color(apply_opacity_to_color(deco.color, opacity)),
            None,
            &rect,
        );
    }

    // 2. Draw glyph runs (with cache)
    let current_frame = FRAME_COUNTER.load(Ordering::Relaxed);
    for run in &payload.prepared.glyph_runs {
        let font_data = run
            .font_data
            .downcast_ref::<peniko::FontData>()
            .expect("font_data is not peniko::FontData");
        let font_id = font_data.data.id();
        let font_size_quanted = (run.font_size * 2.0) as u32;

        // Compute glyph signature: fxhash of glyph ids only
        // (x/y change with position, but glyph ids identify the text content)
        let mut hasher = rustc_hash::FxHasher::default();
        for g in &run.glyphs {
            g.id.hash(&mut hasher);
        }
        let glyph_signature = hasher.finish();
        let effective_color = apply_opacity_to_color(run.color, opacity);

        let cache_key = GlyphRunCacheKey {
            font_ptr: font_id as usize,
            font_size_quanted,
            color: effective_color,
            glyph_signature,
        };

        let mut cache = glyph_run_cache.lock().unwrap();
        if let Some(entry) = cache.get_mut(&cache_key) {
            // Cache hit: reuse pre-built glyphs
            entry
                .last_used_frame
                .store(current_frame, Ordering::Relaxed);
            let glyphs = entry.glyphs.iter().cloned();
            scene
                .draw_glyphs(font_data)
                .brush(Brush::Solid(neutral_to_peniko_color(effective_color)))
                .hint(true)
                .transform(transform)
                .font_size(run.font_size)
                .draw(Fill::NonZero, glyphs);
            drop(cache);
            glyph_run_cache_stats.lock().unwrap().hits += 1;
        } else {
            drop(cache);
            glyph_run_cache_stats.lock().unwrap().misses += 1;

            // Cache miss: build glyphs
            let glyphs: Vec<vello::Glyph> = run
                .glyphs
                .iter()
                .map(|g| vello::Glyph {
                    id: g.id,
                    x: g.x,
                    y: g.y,
                })
                .collect();

            scene
                .draw_glyphs(font_data)
                .brush(Brush::Solid(neutral_to_peniko_color(effective_color)))
                .hint(true)
                .transform(transform)
                .font_size(run.font_size)
                .draw(Fill::NonZero, glyphs.iter().cloned());

            // Only cache if under size limit to prevent HashMap bloat
            let mut cache = glyph_run_cache.lock().unwrap();
            if cache.len() < 1000 {
                let entry = GlyphRunCacheEntry {
                    glyphs,
                    last_used_frame: AtomicU64::new(current_frame),
                };
                cache.insert(cache_key, entry);
            }
        }
    }
}
