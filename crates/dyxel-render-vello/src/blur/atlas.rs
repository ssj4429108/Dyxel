// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pure atlas layout helpers for batching legacy-correct blur textures.

use super::dirty::blur_entry_visible;
use super::types::{BlurAtlasLayout, BlurredTextureEntry};

/// Experimental but correctness-preserving path: pack every visible blur
/// backdrop into fixed atlas slots, run one Kawase blur over the atlas, then
/// composite from the blurred atlas. Unlike the rejected full-frame backdrop
/// path, this keeps per-entry backdrop semantics and padding.
pub(crate) const USE_ATLAS_WIDE_BACKDROP_BLUR: bool = false;
pub(crate) const BLUR_ATLAS_LEGACY_GAP_PX: u32 = 2;
// The atlas-wide path blurs the whole atlas texture. Keep a transparent moat
// between fixed slots so Kawase samples at slot edges do not bleed neighboring
// blur cards into each other.
pub(crate) const BLUR_ATLAS_WIDE_GAP_PX: u32 = 32;
const BLUR_ATLAS_MAX_DIM_PX: u32 = 4096;
#[cfg(target_os = "android")]
const BLUR_ATLAS_WIDE_MAX_SLOTS: usize = 24;
#[cfg(not(target_os = "android"))]
const BLUR_ATLAS_WIDE_MAX_SLOTS: usize = 48;
#[cfg(target_os = "android")]
const BLUR_ATLAS_WIDE_MAX_AREA_PX: u64 = 2_500_000;
#[cfg(not(target_os = "android"))]
const BLUR_ATLAS_WIDE_MAX_AREA_PX: u64 = 5_000_000;

#[inline]
fn ceil_sqrt_u32(n: u32) -> u32 {
    if n <= 1 {
        return n.max(1);
    }
    let mut x = (n as f64).sqrt().ceil() as u32;
    while x.saturating_mul(x) < n {
        x += 1;
    }
    x
}

pub(crate) fn compute_blur_atlas_layout(
    entries: &[BlurredTextureEntry],
    viewport_w: u32,
    viewport_h: u32,
    gap: u32,
) -> Option<BlurAtlasLayout> {
    let mut candidates: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            !entry.skipped_due_to_size && blur_entry_visible(entry, viewport_w, viewport_h)
        })
        .map(|(idx, _)| idx)
        .collect();
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by_key(|&idx| entries[idx].view_id);

    let max_entry_extent = candidates
        .iter()
        .map(|&idx| {
            let entry = &entries[idx];
            entry
                .width
                .max(entry.height)
                .saturating_add(gap.saturating_mul(2))
        })
        .max()
        .unwrap_or(0);

    let slot = if max_entry_extent <= 256 {
        256
    } else if max_entry_extent <= 320 {
        320
    } else if max_entry_extent <= 384 {
        384
    } else {
        return None;
    };

    let count = candidates.len() as u32;
    let max_cols = (BLUR_ATLAS_MAX_DIM_PX / slot).max(1);
    let mut cols = ceil_sqrt_u32(count).min(max_cols).max(1);
    let mut rows = count.div_ceil(cols);
    if rows.saturating_mul(slot) > BLUR_ATLAS_MAX_DIM_PX {
        cols = max_cols;
        rows = count.div_ceil(cols);
    }
    if rows.saturating_mul(slot) > BLUR_ATLAS_MAX_DIM_PX {
        return None;
    }

    let width = cols.saturating_mul(slot);
    let height = rows.saturating_mul(slot);
    let mut placements = Vec::with_capacity(candidates.len());
    for (slot_index, idx) in candidates.into_iter().enumerate() {
        let entry = &entries[idx];
        if entry.width.saturating_add(gap.saturating_mul(2)) > slot
            || entry.height.saturating_add(gap.saturating_mul(2)) > slot
        {
            return None;
        }
        let col = (slot_index as u32) % cols;
        let row = (slot_index as u32) / cols;
        placements.push((idx, col * slot + gap, row * slot + gap));
    }

    Some(BlurAtlasLayout {
        width,
        height,
        slot,
        gap,
        placements,
    })
}

#[inline]
pub(crate) fn blur_atlas_wide_layout_within_budget(layout: &BlurAtlasLayout) -> bool {
    layout.placements.len() <= BLUR_ATLAS_WIDE_MAX_SLOTS
        && (layout.width as u64) * (layout.height as u64) <= BLUR_ATLAS_WIDE_MAX_AREA_PX
}
