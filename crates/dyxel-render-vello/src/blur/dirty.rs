// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dirty tracking, quantization, visibility, and texture sizing helpers for blur.

use super::types::{BlurDirtyKind, BlurDirtyStats, BlurredTextureEntry};

/// Number of blur entries that may be fully rebuilt per frame.
/// Acts as a soft budget; entries beyond this limit are deferred to the next frame.
#[cfg(target_os = "macos")]
pub(crate) const MAX_BLUR_REBUILDS_PER_FRAME: usize = 8;
#[cfg(not(any(target_os = "android", target_os = "macos")))]
pub(crate) const MAX_BLUR_REBUILDS_PER_FRAME: usize = 6;
#[cfg(target_os = "android")]
pub(crate) const MAX_BLUR_REBUILDS_PER_FRAME_AT_60HZ: usize = 1;

pub(crate) const BLUR_SOURCE_RECT_EPS_PX: f32 = 1.0;
const BLUR_SOURCE_POS_BUCKET_PX: f32 = 16.0;
const BLUR_SOURCE_SIZE_BUCKET_PX: f32 = 16.0;

pub(crate) const PARAM_DIRTY_RADIUS: u32 = 1 << 0;
pub(crate) const PARAM_DIRTY_STYLE: u32 = 1 << 1;
pub(crate) const PARAM_DIRTY_SRC_X: u32 = 1 << 2;
pub(crate) const PARAM_DIRTY_SRC_Y: u32 = 1 << 3;
pub(crate) const PARAM_DIRTY_SRC_W: u32 = 1 << 4;
pub(crate) const PARAM_DIRTY_SRC_H: u32 = 1 << 5;

/// Experimental full-frame backdrop blur path.
///
/// Disabled: visual result does not match the legacy per-entry backdrop blur.
/// Keep correctness first; optimization must preserve this legacy visual model.
pub(crate) const USE_FULL_FRAME_BACKDROP_BLUR: bool = false;

pub(crate) struct BlurDirtyReport {
    pub(crate) dirty_count: usize,
    pub(crate) stats: BlurDirtyStats,
    pub(crate) max_radius: f32,
}

#[inline]
pub(crate) fn collect_blur_dirty_report(
    entries: &[BlurredTextureEntry],
    viewport_w: u32,
    viewport_h: u32,
) -> BlurDirtyReport {
    let dirty_count = entries
        .iter()
        .filter(|e| e.dirty_kind != BlurDirtyKind::Clean)
        .count();
    let mut stats = BlurDirtyStats::default();

    for entry in entries {
        if entry.skipped_due_to_size {
            stats.skipped += 1;
        }
        if blur_entry_visible(entry, viewport_w, viewport_h) {
            stats.visible += 1;
        }
        if !entry.blur_valid {
            stats.invalid += 1;
        }
        if entry.blur_rebuild_pending {
            stats.pending += 1;
        }
        if entry.dirty_kind == BlurDirtyKind::BlurParamsChanged {
            if entry.param_dirty_bits & PARAM_DIRTY_RADIUS != 0 {
                stats.param_radius += 1;
            }
            if entry.param_dirty_bits & PARAM_DIRTY_STYLE != 0 {
                stats.param_style += 1;
            }
            if entry.param_dirty_bits & PARAM_DIRTY_SRC_X != 0 {
                stats.param_src_x += 1;
            }
            if entry.param_dirty_bits & PARAM_DIRTY_SRC_Y != 0 {
                stats.param_src_y += 1;
            }
            if entry.param_dirty_bits & PARAM_DIRTY_SRC_W != 0 {
                stats.param_src_w += 1;
            }
            if entry.param_dirty_bits & PARAM_DIRTY_SRC_H != 0 {
                stats.param_src_h += 1;
            }
        }
        if entry.dirty_kind == BlurDirtyKind::BackgroundChanged {
            stats.bg_size += 1;
        }
        if entry.dirty_kind == BlurDirtyKind::ChildrenChanged {
            if entry.deferred_children.is_empty() {
                stats.children_list += 1;
            } else {
                stats.children_bounds += 1;
            }
        }
        match entry.dirty_kind {
            BlurDirtyKind::Clean => stats.clean += 1,
            BlurDirtyKind::BackgroundChanged => stats.background += 1,
            BlurDirtyKind::BlurParamsChanged => stats.params += 1,
            BlurDirtyKind::OverlayOnlyChanged => stats.overlay += 1,
            BlurDirtyKind::ChildrenChanged => stats.children += 1,
        }
    }

    let max_radius = entries
        .iter()
        .filter(|e| !e.skipped_due_to_size)
        .map(|e| e.blur_radius)
        .fold(0.0f32, f32::max);

    BlurDirtyReport {
        dirty_count,
        stats,
        max_radius,
    }
}

#[inline]
pub(crate) fn log_blur_dirty_report(
    current_frame: u64,
    entry_count: usize,
    report: &BlurDirtyReport,
) {
    let stats = &report.stats;
    log::info!(
        "[BlurLegacy] Frame {} — {} entries, {} visible, {} dirty, max_radius={:.1}, stats clean={} bg={} params={} overlay={} children={} invalid={} pending={} skipped={} param_bits radius={} style={} x={} y={} w={} h={} bg_size={} child_list={} child_bounds={}",
        current_frame,
        entry_count,
        stats.visible,
        report.dirty_count,
        report.max_radius,
        stats.clean,
        stats.background,
        stats.params,
        stats.overlay,
        stats.children,
        stats.invalid,
        stats.pending,
        stats.skipped,
        stats.param_radius,
        stats.param_style,
        stats.param_src_x,
        stats.param_src_y,
        stats.param_src_w,
        stats.param_src_h,
        stats.bg_size,
        stats.children_list,
        stats.children_bounds,
    );
}

#[inline]
pub(crate) fn kawase_pass_class_for_radius(radius: f32) -> u32 {
    ((radius / 25.0).ceil() as u32).max(2).min(4)
}

#[inline]
pub(crate) fn blur_texture_alloc_extent_px(active_extent: u32) -> u32 {
    // Blur cards in the current workload fit the 256px atlas slot. Allocating
    // the backing texture in coarse buckets keeps the active draw rect exact
    // while avoiding GPU texture churn and visible invalidation when layout
    // animation jitters by a few pixels.
    if active_extent <= 128 {
        128
    } else if active_extent <= 192 {
        192
    } else if active_extent <= 256 {
        256
    } else if active_extent <= 320 {
        320
    } else if active_extent <= 384 {
        384
    } else {
        active_extent.div_ceil(64) * 64
    }
}

#[inline]
pub(crate) fn quantize_blur_pos_px(v: f32) -> f32 {
    (v / BLUR_SOURCE_POS_BUCKET_PX).round() * BLUR_SOURCE_POS_BUCKET_PX
}

#[inline]
pub(crate) fn quantize_blur_size_px(v: f32) -> f32 {
    // Use ceil for sizes to avoid clipping the source/backdrop when a layout
    // dimension falls between buckets.
    (v / BLUR_SOURCE_SIZE_BUCKET_PX).ceil() * BLUR_SOURCE_SIZE_BUCKET_PX
}

#[inline]
pub(crate) fn blur_rect_changed(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    (a.0 - b.0).abs() >= BLUR_SOURCE_RECT_EPS_PX
        || (a.1 - b.1).abs() >= BLUR_SOURCE_RECT_EPS_PX
        || (a.2 - b.2).abs() >= BLUR_SOURCE_RECT_EPS_PX
        || (a.3 - b.3).abs() >= BLUR_SOURCE_RECT_EPS_PX
}

#[inline]
pub(crate) fn blur_entry_visible(
    entry: &BlurredTextureEntry,
    viewport_w: u32,
    viewport_h: u32,
) -> bool {
    let (x, y, w, h) = entry.source_rect;
    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    x < viewport_w as f32 && y < viewport_h as f32 && x + w > 0.0 && y + h > 0.0
}
