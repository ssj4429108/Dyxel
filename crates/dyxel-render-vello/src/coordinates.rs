// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Platform coordinate-system helpers.
//!
//! Dyxel layout uses screen Y-down coordinates. Android's Vello path needs a
//! root transform for the main scene, but blur/offscreen passes should keep
//! their own explicit rules:
//! - Pass 1 scene: use `platform_correction` as the Vello root transform.
//! - Android Pass 2 blur source copy: mirror only the raw copy source Y.
//! - Pass 3 children/offscreen: keep local screen coordinates.
//! - Composite shaders: sample normal local UVs; no extra Y flip.

use kurbo::Affine;

/// Returns the platform-specific coordinate correction transform for the main
/// Vello scene.
#[inline]
pub fn platform_correction(viewport_height: f64) -> Affine {
    #[cfg(target_os = "android")]
    {
        // Android: Vello renders Y-up, need flip to match screen Y-down.
        Affine::translate((0.0, viewport_height)) * Affine::scale_non_uniform(1.0, -1.0)
    }
    #[cfg(not(target_os = "android"))]
    {
        // macOS/iOS: Vello's render_to_texture already produces Y-down output.
        let _ = viewport_height;
        Affine::IDENTITY
    }
}
