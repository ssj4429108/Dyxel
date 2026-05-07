// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Color conversion helpers shared by Vello scene building and effect passes.

use vello::peniko;

#[inline]
pub(crate) fn neutral_to_peniko_color(c: [u8; 4]) -> peniko::Color {
    peniko::Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Apply opacity to a neutral [u8; 4] color by scaling only the alpha channel.
/// Colors are non-premultiplied sRGB; opacity affects the final alpha only.
#[inline]
pub(crate) fn apply_opacity_to_color(c: [u8; 4], opacity: f32) -> [u8; 4] {
    if opacity >= 1.0 {
        c
    } else {
        let alpha = opacity.clamp(0.0, 1.0);
        [c[0], c[1], c[2], (c[3] as f32 * alpha) as u8]
    }
}
