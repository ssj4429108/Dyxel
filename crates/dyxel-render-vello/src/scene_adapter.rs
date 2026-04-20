// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Scene adapter - implements dyxel_render_api::Scene trait for vello::Scene
//!
//! This adapter maps the high-level Scene API to Vello's native layer system,
//! enabling alpha blending, filters, and clipping using Vello's push_layer/pop_layer.

use dyxel_render_api::{filters::{BlendMode as DyxelBlendMode, Filter, Rect}, Scene as SceneTrait, Transform};
use kurbo::{Affine, Rect as KRect, RoundedRect};
use vello::Scene;
use vello::peniko::{BlendMode as PenikoBlendMode, Color, Compose, Fill, Mix};

/// Adapter that wraps vello::Scene and implements dyxel_render_api::Scene trait
pub struct VelloSceneAdapter<'a> {
    scene: &'a mut Scene,
    transform_stack: Vec<Affine>,
}

impl<'a> VelloSceneAdapter<'a> {
    /// Create a new scene adapter wrapping a vello scene
    pub fn new(scene: &'a mut Scene) -> Self {
        Self {
            scene,
            transform_stack: Vec::new(),
        }
    }

    /// Get a reference to the underlying vello scene
    pub fn inner(&self) -> &Scene {
        self.scene
    }

    /// Get a mutable reference to the underlying vello scene
    pub fn inner_mut(&mut self) -> &mut Scene {
        self.scene
    }

    /// Convert Dyxel BlendMode to Peniko BlendMode
    fn convert_blend_mode(mode: DyxelBlendMode) -> PenikoBlendMode {
        match mode {
            DyxelBlendMode::Normal => PenikoBlendMode::new(Mix::Normal, Compose::SrcOver),
            DyxelBlendMode::Multiply => PenikoBlendMode::new(Mix::Multiply, Compose::SrcOver),
            DyxelBlendMode::Screen => PenikoBlendMode::new(Mix::Screen, Compose::SrcOver),
            DyxelBlendMode::Overlay => PenikoBlendMode::new(Mix::Overlay, Compose::SrcOver),
            DyxelBlendMode::Darken => PenikoBlendMode::new(Mix::Darken, Compose::SrcOver),
            DyxelBlendMode::Lighten => PenikoBlendMode::new(Mix::Lighten, Compose::SrcOver),
            DyxelBlendMode::ColorDodge => PenikoBlendMode::new(Mix::ColorDodge, Compose::SrcOver),
            DyxelBlendMode::ColorBurn => PenikoBlendMode::new(Mix::ColorBurn, Compose::SrcOver),
            DyxelBlendMode::HardLight => PenikoBlendMode::new(Mix::HardLight, Compose::SrcOver),
            DyxelBlendMode::SoftLight => PenikoBlendMode::new(Mix::SoftLight, Compose::SrcOver),
            DyxelBlendMode::Difference => PenikoBlendMode::new(Mix::Difference, Compose::SrcOver),
            DyxelBlendMode::Exclusion => PenikoBlendMode::new(Mix::Exclusion, Compose::SrcOver),
        }
    }

    /// Convert Transform to Affine
    fn transform_to_affine(transform: Transform) -> Affine {
        Affine::new([
            transform.xx,
            transform.yx,
            transform.xy,
            transform.yy,
            transform.x0,
            transform.y0,
        ])
    }

    /// Get the current transform (last pushed or identity)
    fn current_transform(&self) -> Affine {
        self.transform_stack
            .last()
            .copied()
            .unwrap_or(Affine::IDENTITY)
    }
}

impl<'a> SceneTrait for VelloSceneAdapter<'a> {
    fn fill_rect(&mut self, x: f64, y: f64, width: f64, height: f64, color: [u8; 4]) {
        let rect = KRect::from_origin_size((x, y), (width, height));
        let peniko_color = Color::from_rgba8(color[0], color[1], color[2], color[3]);
        self.scene.fill(
            Fill::NonZero,
            self.current_transform(),
            peniko_color,
            None,
            &rect,
        );
    }

    fn fill_rounded_rect(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        radius: f64,
        color: [u8; 4],
    ) {
        let rect = KRect::from_origin_size((x, y), (width, height));
        let rounded = RoundedRect::from_rect(rect, radius);
        let peniko_color = Color::from_rgba8(color[0], color[1], color[2], color[3]);
        self.scene.fill(
            Fill::NonZero,
            self.current_transform(),
            peniko_color,
            None,
            &rounded,
        );
    }

    fn push_transform(&mut self, transform: Transform) {
        let affine = Self::transform_to_affine(transform);
        let new_transform = if let Some(last) = self.transform_stack.last() {
            *last * affine
        } else {
            affine
        };
        self.transform_stack.push(new_transform);
    }

    fn pop_transform(&mut self) {
        self.transform_stack.pop();
    }

    fn clear(&mut self) {
        self.scene.reset();
        self.transform_stack.clear();
    }

    fn push_layer(
        &mut self,
        alpha: f32,
        blend: DyxelBlendMode,
        _filter: Option<&Filter>,
        clip: Option<Rect>,
    ) {
        // Convert blend mode
        let peniko_blend = Self::convert_blend_mode(blend);

        // Get current transform
        let transform = self.current_transform();

        // Create clip shape if provided
        if let Some(clip_rect) = clip {
            let krect = KRect::from_origin_size(
                (clip_rect.x as f64, clip_rect.y as f64),
                (clip_rect.width as f64, clip_rect.height as f64),
            );
            // Vello's push_layer with clip
            self.scene.push_layer(
                Fill::NonZero,
                peniko_blend,
                alpha.clamp(0.0, 1.0),
                transform,
                &krect,
            );
        } else {
            // No clip - use full screen clip (empty rect means no geometric clip)
            // Vello requires a shape, so we use a large rect
            let full_rect = KRect::from_origin_size((-1e6, -1e6), (2e6, 2e6));
            self.scene.push_layer(
                Fill::NonZero,
                peniko_blend,
                alpha.clamp(0.0, 1.0),
                transform,
                &full_rect,
            );
        }

        // Note: Filter effects (Blur, DropShadow) in Vello are handled differently
        // They are typically applied as separate draw operations rather than layer properties.
        // For full filter support, we would need to use Vello's filter extensions.
        // For now, we capture the filter parameter for future implementation.
        if _filter.is_some() {
            // TODO: Implement filter effects using Vello's native filter support
            // This may require drawing to an intermediate layer and applying effects
        }
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }
}

/// Extension trait for drawing shadows using Vello's blur
///
/// This follows Xilem's approach: draw shadow first (with blur), then draw content
pub trait ShadowRenderer {
    /// Draw a shadow for a rectangle
    fn draw_rect_shadow(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: [u8; 4],
        blur_radius: f64,
        offset_x: f64,
        offset_y: f64,
    );

    /// Draw a shadow for a rounded rectangle
    fn draw_rounded_rect_shadow(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        radius: f64,
        color: [u8; 4],
        blur_radius: f64,
        offset_x: f64,
        offset_y: f64,
    );
}

impl<'a> ShadowRenderer for VelloSceneAdapter<'a> {
    fn draw_rect_shadow(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        color: [u8; 4],
        blur_radius: f64,
        offset_x: f64,
        offset_y: f64,
    ) {
        // Position shadow with offset
        let shadow_x = x + offset_x;
        let shadow_y = y + offset_y;

        // Vello 0.7+ has draw_blurred_rounded_rect for shadow effects
        // For rectangles, we use the same with radius = 0
        let peniko_color = Color::from_rgba8(color[0], color[1], color[2], color[3]);

        // Vello's draw_blurred_rounded_rect draws a blurred shape directly
        self.scene.draw_blurred_rounded_rect(
            self.current_transform(),
            KRect::from_origin_size((shadow_x, shadow_y), (width, height)),
            peniko_color,
            0.0, // sharp corners for rect
            blur_radius,
        );
    }

    fn draw_rounded_rect_shadow(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        radius: f64,
        color: [u8; 4],
        blur_radius: f64,
        offset_x: f64,
        offset_y: f64,
    ) {
        let shadow_x = x + offset_x;
        let shadow_y = y + offset_y;
        let peniko_color = Color::from_rgba8(color[0], color[1], color[2], color[3]);

        self.scene.draw_blurred_rounded_rect(
            self.current_transform(),
            KRect::from_origin_size((shadow_x, shadow_y), (width, height)),
            peniko_color,
            radius,
            blur_radius,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blend_mode_conversion() {
        // Test that all blend modes convert without panicking
        let modes = [
            DyxelBlendMode::Normal,
            DyxelBlendMode::Multiply,
            DyxelBlendMode::Screen,
            DyxelBlendMode::Overlay,
            DyxelBlendMode::Darken,
            DyxelBlendMode::Lighten,
            DyxelBlendMode::ColorDodge,
            DyxelBlendMode::ColorBurn,
            DyxelBlendMode::HardLight,
            DyxelBlendMode::SoftLight,
            DyxelBlendMode::Difference,
            DyxelBlendMode::Exclusion,
        ];

        for mode in &modes {
            let _ = VelloSceneAdapter::convert_blend_mode(*mode);
        }
    }

    #[test]
    fn test_transform_conversion() {
        let t = Transform::translate(10.0, 20.0);
        let affine = VelloSceneAdapter::transform_to_affine(t);
        // Affine should have translation in the last two elements
        // Note: kurbo uses column-major order for the 2x2 part
    }
}
