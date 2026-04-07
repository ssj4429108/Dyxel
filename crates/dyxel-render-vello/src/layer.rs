// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Render Layer - Xilem-inspired layer-based rendering
//!
//! This module implements a layer-based rendering architecture inspired by Xilem:
//! - Each offscreen element is a Layer with its own texture
//! - Effects (shadow, blur) are composed as layers
//! - Caching is explicit and controllable

use std::sync::Arc;
use vello::{kurbo::Affine, peniko::Color, Scene};

/// A render layer represents an offscreen render target
/// Similar to Xilem's concept of cached layers
pub struct Layer {
    /// Unique layer identifier
    pub id: LayerId,
    /// The Vello scene for this layer
    pub scene: Scene,
    /// Transform from layer-local to parent coordinates
    pub transform: Affine,
    /// Layer bounds in parent coordinates
    pub bounds: LayerBounds,
    /// Opacity (0.0 - 1.0)
    pub opacity: f32,
    /// Effects to apply (shadow, blur, etc.)
    pub effects: Vec<Effect>,
    /// Whether this layer needs re-rendering
    pub is_dirty: bool,
    /// Cache hint for eviction policy
    pub cache_hint: CacheHint,
}

/// Layer identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LayerId(pub u64);

impl LayerId {
    /// Generate next layer ID
    pub fn next(&self) -> Self {
        LayerId(self.0 + 1)
    }
}

/// Layer bounds in screen/parent coordinates
#[derive(Debug, Clone, Copy, Default)]
pub struct LayerBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LayerBounds {
    /// Create new bounds
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width: f32::max(width, 1.0),
            height: f32::max(height, 1.0),
        }
    }

    /// Convert to kurbo Rect
    pub fn to_kurbo_rect(&self) -> vello::kurbo::Rect {
        vello::kurbo::Rect::new(
            self.x as f64,
            self.y as f64,
            (self.x + self.width) as f64,
            (self.y + self.height) as f64,
        )
    }

    /// Check if point is inside bounds
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x
            && x <= self.x + self.width
            && y >= self.y
            && y <= self.y + self.height
    }

    /// Expand bounds by padding (for effects like shadow)
    pub fn expand(&self, padding: f32) -> Self {
        Self::new(
            self.x - padding,
            self.y - padding,
            self.width + padding * 2.0,
            self.height + padding * 2.0,
        )
    }
}

/// Effect types for layers
#[derive(Debug, Clone, Copy)]
pub enum Effect {
    /// Gaussian blur
    Blur { radius: f32 },
    /// Drop shadow
    Shadow {
        dx: f32,
        dy: f32,
        blur_radius: f32,
        color: Color,
    },
    /// Blend mode for compositing
    BlendMode(BlendMode),
}

/// Blend modes for layer compositing
#[derive(Debug, Clone, Copy, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Plus,
}

/// Cache hint for layer eviction policy
#[derive(Debug, Clone, Copy, Default)]
pub enum CacheHint {
    /// Don't cache, render every frame
    Never,
    /// Cache but allow eviction under memory pressure
    #[default]
    Auto,
    /// Keep cached, prefer not to evict
    Persistent,
}

impl Layer {
    /// Create a new layer
    pub fn new(id: LayerId, bounds: LayerBounds) -> Self {
        Self {
            id,
            scene: Scene::new(),
            transform: Affine::default(),
            bounds,
            opacity: 1.0,
            effects: Vec::new(),
            is_dirty: true,
            cache_hint: CacheHint::default(),
        }
    }

    /// Set layer opacity
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Add an effect
    pub fn with_effect(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }

    /// Set cache hint
    pub fn with_cache_hint(mut self, hint: CacheHint) -> Self {
        self.cache_hint = hint;
        self
    }

    /// Mark layer as dirty (needs re-render)
    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }

    /// Mark layer as clean (rendered)
    pub fn mark_clean(&mut self) {
        self.is_dirty = false;
    }

    /// Get the required texture size including effect padding
    pub fn texture_size(&self) -> (u32, u32) {
        let mut bounds = self.bounds;

        // Expand bounds for shadow effects
        for effect in &self.effects {
            if let Effect::Shadow {
                dx,
                dy,
                blur_radius,
                ..
            } = effect
            {
                let padding = blur_radius + dx.abs().max(dy.abs());
                bounds = bounds.expand(padding);
            }
        }

        (
            bounds.width.ceil() as u32,
            bounds.height.ceil() as u32,
        )
    }

    /// Calculate the texture-to-screen transform
    /// This accounts for effect padding offset
    pub fn texture_transform(&self) -> Affine {
        let (tex_width, tex_height) = self.texture_size();
        let offset = self.texture_offset();

        Affine::translate((-offset.x as f64, -offset.y as f64))
            * Affine::scale_non_uniform(
                self.bounds.width as f64 / tex_width as f64,
                self.bounds.height as f64 / tex_height as f64,
            )
    }

    /// Get texture offset due to effect padding
    pub fn texture_offset(&self) -> LayerBounds {
        let mut offset_x = 0.0f32;
        let mut offset_y = 0.0f32;

        for effect in &self.effects {
            if let Effect::Shadow { dx, dy, .. } = effect {
                offset_x = offset_x.min(*dx);
                offset_y = offset_y.min(*dy);
            }
        }

        LayerBounds::new(offset_x, offset_y, 0.0, 0.0)
    }
}

/// Layer manager for organizing and caching layers
pub struct LayerManager {
    layers: std::collections::HashMap<LayerId, Layer>,
    next_id: u64,
}

impl LayerManager {
    /// Create a new layer manager
    pub fn new() -> Self {
        Self {
            layers: std::collections::HashMap::new(),
            next_id: 1,
        }
    }

    /// Create a new layer
    pub fn create_layer(&mut self, bounds: LayerBounds) -> &mut Layer {
        let id = LayerId(self.next_id);
        self.next_id += 1;

        let layer = Layer::new(id, bounds);
        self.layers.insert(id, layer);
        self.layers.get_mut(&id).unwrap()
    }

    /// Get a layer by ID
    pub fn get(&self, id: LayerId) -> Option<&Layer> {
        self.layers.get(&id)
    }

    /// Get a mutable layer by ID
    pub fn get_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.get_mut(&id)
    }

    /// Remove a layer
    pub fn remove(&mut self, id: LayerId) -> Option<Layer> {
        self.layers.remove(&id)
    }

    /// Get all dirty layers that need re-rendering
    pub fn dirty_layers(&self) -> Vec<&Layer> {
        self.layers.values().filter(|l| l.is_dirty).collect()
    }

    /// Mark all layers as dirty (e.g., after resize)
    pub fn mark_all_dirty(&mut self) {
        for layer in self.layers.values_mut() {
            layer.mark_dirty();
        }
    }

    /// Clear all layers
    pub fn clear(&mut self) {
        self.layers.clear();
        self.next_id = 1;
    }
}

impl Default for LayerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_bounds() {
        let bounds = LayerBounds::new(10.0, 20.0, 100.0, 200.0);
        assert_eq!(bounds.x, 10.0);
        assert_eq!(bounds.y, 20.0);
        assert_eq!(bounds.width, 100.0);
        assert_eq!(bounds.height, 200.0);
    }

    #[test]
    fn test_layer_bounds_contains() {
        let bounds = LayerBounds::new(0.0, 0.0, 100.0, 100.0);
        assert!(bounds.contains(50.0, 50.0));
        assert!(!bounds.contains(150.0, 50.0));
    }

    #[test]
    fn test_layer_bounds_expand() {
        let bounds = LayerBounds::new(0.0, 0.0, 100.0, 100.0);
        let expanded = bounds.expand(10.0);
        assert_eq!(expanded.x, -10.0);
        assert_eq!(expanded.y, -10.0);
        assert_eq!(expanded.width, 120.0);
        assert_eq!(expanded.height, 120.0);
    }

    #[test]
    fn test_layer_texture_size_with_shadow() {
        let mut layer = Layer::new(LayerId(1), LayerBounds::new(0.0, 0.0, 100.0, 100.0));
        layer.effects.push(Effect::Shadow {
            dx: 5.0,
            dy: 5.0,
            blur_radius: 10.0,
            color: Color::BLACK,
        });

        let (w, h) = layer.texture_size();
        // Should be expanded by blur_radius + max(dx, dy) = 10 + 5 = 15 on each side
        assert!(w > 100);
        assert!(h > 100);
    }

    #[test]
    fn test_layer_manager() {
        let mut manager = LayerManager::new();
        let layer = manager.create_layer(LayerBounds::new(0.0, 0.0, 100.0, 100.0));
        let id = layer.id;

        assert!(manager.get(id).is_some());
        assert_eq!(manager.dirty_layers().len(), 1);

        manager.get_mut(id).unwrap().mark_clean();
        assert_eq!(manager.dirty_layers().len(), 0);
    }
}
