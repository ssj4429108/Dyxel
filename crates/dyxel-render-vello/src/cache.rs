// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Raster-cache draw command types used by the Vello backend.

use crate::texture_pool;
use kurbo::Affine;

/// A cached subtree draw command for the post-scene blit pass.
#[derive(Debug)]
pub(crate) struct CachedDraw {
    /// The cached texture identifier.
    pub(crate) texture_id: texture_pool::TextureId,
    /// Transform that positions the texture on screen.
    pub(crate) transform: Affine,
    /// Width of the cached texture in pixels.
    pub(crate) width: f32,
    /// Height of the cached texture in pixels.
    pub(crate) height: f32,
}
