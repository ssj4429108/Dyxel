// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! WASM-side Filter Effects API
//!
//! Provides GPU-accelerated filter effects like Gaussian blur and drop shadow
//! for off-screen rendered layers.

use crate::View;
use dyxel_shared::push_command;

/// Filter identifier (16-bit to match protocol)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FilterId(pub u16);

impl FilterId {
    /// Create a new filter ID
    pub const fn new(id: u16) -> Self {
        Self(id)
    }
}

/// Blend mode for layer composition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlendMode {
    /// Normal alpha blending (default)
    Normal = 0,
    /// Multiply blend
    Multiply = 1,
    /// Screen blend
    Screen = 2,
    /// Overlay blend
    Overlay = 3,
    /// Darken blend
    Darken = 4,
    /// Lighten blend
    Lighten = 5,
    /// Color dodge
    ColorDodge = 6,
    /// Color burn
    ColorBurn = 7,
    /// Hard light
    HardLight = 8,
    /// Soft light
    SoftLight = 9,
    /// Difference
    Difference = 10,
    /// Exclusion
    Exclusion = 11,
}

impl Default for BlendMode {
    fn default() -> Self {
        BlendMode::Normal
    }
}

/// Filter definition for GPU effects
#[derive(Debug, Clone)]
pub enum Filter {
    /// Gaussian blur filter
    Blur {
        /// Horizontal blur radius in pixels
        radius_x: f32,
        /// Vertical blur radius in pixels
        radius_y: f32,
    },
    /// Drop shadow filter
    DropShadow {
        /// Horizontal offset
        dx: f32,
        /// Vertical offset
        dy: f32,
        /// Blur radius
        blur_radius: f32,
        /// Shadow color (RGBA)
        color: u32,
    },
    /// Combined filter chain
    Combined(Vec<Filter>),
}

impl Filter {
    /// Create a simple Gaussian blur filter
    pub fn blur(radius: f32) -> Self {
        Filter::Blur {
            radius_x: radius,
            radius_y: radius,
        }
    }

    /// Create an asymmetric Gaussian blur filter
    pub fn blur_asymmetric(radius_x: f32, radius_y: f32) -> Self {
        Filter::Blur {
            radius_x,
            radius_y,
        }
    }

    /// Create a drop shadow filter
    pub fn drop_shadow(dx: f32, dy: f32, blur_radius: f32, color: u32) -> Self {
        Filter::DropShadow {
            dx,
            dy,
            blur_radius,
            color,
        }
    }

    /// Create a drop shadow with default blur
    pub fn shadow_simple(dx: f32, dy: f32, color: u32) -> Self {
        Filter::DropShadow {
            dx,
            dy,
            blur_radius: 10.0,
            color,
        }
    }

    /// Chain multiple filters
    pub fn combine(filters: Vec<Filter>) -> Self {
        Filter::Combined(filters)
    }
}

/// Global filter registry for managing filter definitions
pub struct FilterRegistry;

impl FilterRegistry {
    /// Define a blur filter in the global registry
    pub fn define_blur(filter_id: FilterId, radius_x: f32, radius_y: f32) {
        push_command!(
            crate::SHARED_BUFFER,
            DefineFilterBlur,
            filter_id.0,
            radius_x,
            radius_y
        );
    }

    /// Define a drop shadow filter
    pub fn define_drop_shadow(
        filter_id: FilterId,
        dx: f32,
        dy: f32,
        blur_radius: f32,
        color: u32,
    ) {
        push_command!(
            crate::SHARED_BUFFER,
            DefineFilterDropShadow,
            filter_id.0,
            dx,
            dy,
            blur_radius,
            color
        );
    }
}

/// Extension trait for View to support filter effects
pub trait ViewFilterExt {
    /// Apply a filter to this view's off-screen layer
    ///
    /// The view must already have off-screen rendering enabled.
    /// The filter ID must have been previously defined using FilterRegistry.
    fn with_filter(self, filter_id: FilterId, blend_mode: BlendMode) -> Self;

    /// Create an off-screen layer with a filter applied
    ///
    /// This is a convenience method that combines offscreen() and with_filter().
    fn offscreen_with_filter(
        self,
        alpha: f32,
        filter_id: FilterId,
        blend_mode: BlendMode,
    ) -> Self;

    /// Set the blend mode for this view's layer
    fn blend_mode(self, mode: BlendMode) -> Self;
}

impl ViewFilterExt for View {
    fn with_filter(self, filter_id: FilterId, blend_mode: BlendMode) -> Self {
        let id = self.id;
        let (width, height) = self.get_layout_estimated_size();

        // Use PopLayer to end the previous filter scope if needed
        // This is handled by the Host's layer stack management
        push_command!(
            crate::SHARED_BUFFER,
            PopFilter
        );

        self
    }

    fn offscreen_with_filter(
        self,
        alpha: f32,
        filter_id: FilterId,
        blend_mode: BlendMode,
    ) -> Self {
        let id = self.id;
        let (width, height) = self.get_layout_estimated_size();

        // Push layer with filter
        // Note: bounds_x/y are set to 0.0 here, actual positions come from Taffy layout in Host
        push_command!(
            crate::SHARED_BUFFER,
            PushLayerWithFilter,
            id,
            0.0f32, // bounds_x - will be updated from Taffy layout in Host
            0.0f32, // bounds_y - will be updated from Taffy layout in Host
            width,
            height,
            alpha.clamp(0.0, 1.0),
            filter_id.0,
            blend_mode as u8
        );

        self
    }

    fn blend_mode(self, mode: BlendMode) -> Self {
        let id = self.id;
        push_command!(
            crate::SHARED_BUFFER,
            SetBlendMode,
            id,
            mode as u8
        );
        self
    }
}

/// Internal helper trait for layout estimation
trait LayoutEstimation {
    fn get_layout_estimated_size(&self) -> (f32, f32);
}

impl LayoutEstimation for View {
    fn get_layout_estimated_size(&self) -> (f32, f32) {
        // Return default size (Host layout will compute actual bounds)
        (100.0, 100.0)
    }
}

/// Predefined filter presets for common effects
pub mod presets {
    use super::FilterId;

    // Common filter IDs (user-defined filters start from 100)
    pub const BLUR_LIGHT: FilterId = FilterId(1);
    pub const BLUR_MEDIUM: FilterId = FilterId(2);
    pub const BLUR_HEAVY: FilterId = FilterId(3);
    pub const SHADOW_SOFT: FilterId = FilterId(10);
    pub const SHADOW_HARD: FilterId = FilterId(11);
    pub const SHADOW_COLORED: FilterId = FilterId(12);

    /// Register all preset filters
    pub fn register_presets() {
        // Light blur (2px radius)
        super::FilterRegistry::define_blur(BLUR_LIGHT, 2.0, 2.0);
        // Medium blur (5px radius)
        super::FilterRegistry::define_blur(BLUR_MEDIUM, 5.0, 5.0);
        // Heavy blur (10px radius)
        super::FilterRegistry::define_blur(BLUR_HEAVY, 10.0, 10.0);
        // Soft shadow
        super::FilterRegistry::define_drop_shadow(SHADOW_SOFT, 0.0, 4.0, 8.0, 0x40000000);
        // Hard shadow
        super::FilterRegistry::define_drop_shadow(SHADOW_HARD, 2.0, 2.0, 2.0, 0x80000000);
        // Colored shadow
        super::FilterRegistry::define_drop_shadow(SHADOW_COLORED, 0.0, 6.0, 12.0, 0x40FF6B6B);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_creation() {
        let blur = Filter::blur(5.0);
        match blur {
            Filter::Blur { radius_x, radius_y } => {
                assert_eq!(radius_x, 5.0);
                assert_eq!(radius_y, 5.0);
            }
            _ => panic!("Expected Blur filter"),
        }
    }

    #[test]
    fn test_blur_asymmetric() {
        let blur = Filter::blur_asymmetric(2.0, 8.0);
        match blur {
            Filter::Blur { radius_x, radius_y } => {
                assert_eq!(radius_x, 2.0);
                assert_eq!(radius_y, 8.0);
            }
            _ => panic!("Expected Blur filter"),
        }
    }

    #[test]
    fn test_drop_shadow() {
        let shadow = Filter::drop_shadow(2.0, 4.0, 8.0, 0xFF000000);
        match shadow {
            Filter::DropShadow { dx, dy, blur_radius, color } => {
                assert_eq!(dx, 2.0);
                assert_eq!(dy, 4.0);
                assert_eq!(blur_radius, 8.0);
                assert_eq!(color, 0xFF000000);
            }
            _ => panic!("Expected DropShadow filter"),
        }
    }

    #[test]
    fn test_blend_mode_values() {
        assert_eq!(BlendMode::Normal as u8, 0);
        assert_eq!(BlendMode::Multiply as u8, 1);
        assert_eq!(BlendMode::Screen as u8, 2);
        assert_eq!(BlendMode::Exclusion as u8, 11);
    }

    #[test]
    fn test_filter_id() {
        let id = FilterId::new(42);
        assert_eq!(id.0, 42);
    }
}
