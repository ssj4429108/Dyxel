// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Filter types for off-screen rendering
//!
//! These types are shared between Guest (WASM) and Host (native).
//! They define the filter effects protocol for GPU-accelerated rendering.

use std::collections::HashMap;

/// Filter type identifier for protocol communication
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterType {
    /// No filter applied
    None = 0,
    /// Gaussian/Kawase blur filter
    Blur = 1,
    /// Color matrix transformation
    ColorMatrix = 2,
    /// Drop shadow effect
    DropShadow = 3,
}

impl FilterType {
    /// Convert from u32
    pub fn from_u32(value: u32) -> Self {
        match value {
            1 => FilterType::Blur,
            2 => FilterType::ColorMatrix,
            3 => FilterType::DropShadow,
            _ => FilterType::None,
        }
    }
}

/// Blend mode for layer composition
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl BlendMode {
    /// Convert from u8
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => BlendMode::Normal,
            1 => BlendMode::Multiply,
            2 => BlendMode::Screen,
            3 => BlendMode::Overlay,
            4 => BlendMode::Darken,
            5 => BlendMode::Lighten,
            6 => BlendMode::ColorDodge,
            7 => BlendMode::ColorBurn,
            8 => BlendMode::HardLight,
            9 => BlendMode::SoftLight,
            10 => BlendMode::Difference,
            11 => BlendMode::Exclusion,
            _ => BlendMode::Normal,
        }
    }

    /// Check if this blend mode requires an extra pass
    pub fn needs_extra_pass(&self) -> bool {
        matches!(
            self,
            BlendMode::Multiply
                | BlendMode::Screen
                | BlendMode::Overlay
                | BlendMode::Darken
                | BlendMode::Lighten
                | BlendMode::ColorDodge
                | BlendMode::ColorBurn
                | BlendMode::HardLight
                | BlendMode::SoftLight
                | BlendMode::Difference
                | BlendMode::Exclusion
        )
    }
}

impl Default for BlendMode {
    fn default() -> Self {
        BlendMode::Normal
    }
}

/// Layer attributes for off-screen rendering
/// This struct is passed across the FFI boundary
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LayerAttribute {
    /// Filter type to apply
    pub filter_type: FilterType,
    /// Blur radius (for Blur filter)
    pub blur_radius: f32,
    /// Color matrix (5x4 matrix for ColorMatrix filter, stored row-major)
    pub matrix: [f32; 20],
    /// Layer opacity (0.0 - 1.0)
    pub opacity: f32,
    /// Blend mode for compositing
    pub blend_mode: u32,
}

impl Default for LayerAttribute {
    fn default() -> Self {
        Self {
            filter_type: FilterType::None,
            blur_radius: 0.0,
            matrix: [
                1.0, 0.0, 0.0, 0.0, // Red
                0.0, 1.0, 0.0, 0.0, // Green
                0.0, 0.0, 1.0, 0.0, // Blue
                0.0, 0.0, 0.0, 1.0, // Alpha
                0.0, 0.0, 0.0, 0.0, // Bias
            ],
            opacity: 1.0,
            blend_mode: BlendMode::Normal as u32,
        }
    }
}

impl LayerAttribute {
    /// Create a new layer attribute with normal blending
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with blur filter
    pub fn with_blur(radius: f32) -> Self {
        Self {
            filter_type: FilterType::Blur,
            blur_radius: radius,
            ..Default::default()
        }
    }

    /// Create with opacity
    pub fn with_alpha(alpha: f32) -> Self {
        Self {
            opacity: alpha.clamp(0.0, 1.0),
            ..Default::default()
        }
    }

    /// Create with blend mode
    pub fn with_blend_mode(mode: BlendMode) -> Self {
        Self {
            blend_mode: mode as u32,
            ..Default::default()
        }
    }

    /// Set filter type
    pub fn with_filter_type(mut self, filter_type: FilterType) -> Self {
        self.filter_type = filter_type;
        self
    }
}

/// Filter identifier (16-bit to match protocol)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FilterId(pub u16);

impl FilterId {
    /// Create a new filter ID
    pub const fn new(id: u16) -> Self {
        Self(id)
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
    /// Color matrix filter
    ColorMatrix {
        /// 5x4 color matrix (row-major)
        matrix: [f32; 20],
    },
    /// Combination of multiple filters
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
        Filter::Blur { radius_x, radius_y }
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

    /// Create a color matrix filter
    pub fn color_matrix(matrix: [f32; 20]) -> Self {
        Filter::ColorMatrix { matrix }
    }

    /// Create an identity color matrix (no change)
    pub fn identity_matrix() -> [f32; 20] {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0,
        ]
    }

    /// Chain multiple filters
    pub fn combine(filters: Vec<Filter>) -> Self {
        Filter::Combined(filters)
    }

    /// Get the effective bounds expansion required for this filter
    /// Returns (left, top, right, bottom) padding in pixels
    pub fn bounds_expansion(&self) -> (f32, f32, f32, f32) {
        match self {
            Filter::Blur { radius_x, radius_y } => (*radius_x, *radius_y, *radius_x, *radius_y),
            Filter::DropShadow {
                dx,
                dy,
                blur_radius,
                ..
            } => {
                let expand = *blur_radius;
                let left = if *dx < 0.0 { expand - dx } else { expand };
                let right = if *dx > 0.0 { expand + dx } else { expand };
                let top = if *dy < 0.0 { expand - dy } else { expand };
                let bottom = if *dy > 0.0 { expand + dy } else { expand };
                (left, top, right, bottom)
            }
            Filter::ColorMatrix { .. } => (0.0, 0.0, 0.0, 0.0),
            Filter::Combined(filters) => {
                let mut total: (f32, f32, f32, f32) = (0.0, 0.0, 0.0, 0.0);
                for filter in filters {
                    let (l, t, r, b) = filter.bounds_expansion();
                    total.0 = total.0.max(l);
                    total.1 = total.1.max(t);
                    total.2 = total.2.max(r);
                    total.3 = total.3.max(b);
                }
                total
            }
        }
    }
}

/// Global filter registry for managing filter definitions
#[derive(Debug)]
pub struct FilterRegistry {
    filters: HashMap<FilterId, Filter>,
}

impl FilterRegistry {
    /// Create a new filter registry
    pub fn new() -> Self {
        Self {
            filters: HashMap::new(),
        }
    }

    /// Define a new filter
    pub fn define_filter(&mut self, id: FilterId, filter: Filter) {
        log::debug!("FilterRegistry: Defined filter {:?} -> {:?}", id, filter);
        self.filters.insert(id, filter);
    }

    /// Get a filter by ID
    pub fn get_filter(&self, id: FilterId) -> Option<&Filter> {
        self.filters.get(&id)
    }

    /// Check if a filter exists
    pub fn has_filter(&self, id: FilterId) -> bool {
        self.filters.contains_key(&id)
    }

    /// Remove a filter
    pub fn remove_filter(&mut self, id: FilterId) -> Option<Filter> {
        self.filters.remove(&id)
    }

    /// Clear all filters
    pub fn clear(&mut self) {
        self.filters.clear();
    }

    /// Get the number of defined filters
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

impl Default for FilterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Rectangle for bounds definition
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Create a new rectangle
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Check if a point is inside the rectangle
    pub fn contains(&self, point: (f32, f32)) -> bool {
        point.0 >= self.x
            && point.0 <= self.x + self.width
            && point.1 >= self.y
            && point.1 <= self.y + self.height
    }

    /// Expand the rectangle by the given amount on each side
    pub fn expand(&self, left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            x: self.x - left,
            y: self.y - top,
            width: self.width + left + right,
            height: self.height + top + bottom,
        }
    }

    /// Get the area of the rectangle
    pub fn area(&self) -> f32 {
        self.width * self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_type_from_u32() {
        assert_eq!(FilterType::from_u32(0), FilterType::None);
        assert_eq!(FilterType::from_u32(1), FilterType::Blur);
        assert_eq!(FilterType::from_u32(2), FilterType::ColorMatrix);
        assert_eq!(FilterType::from_u32(3), FilterType::DropShadow);
        assert_eq!(FilterType::from_u32(99), FilterType::None);
    }

    #[test]
    fn test_blend_mode_from_u8() {
        assert_eq!(BlendMode::from_u8(0), BlendMode::Normal);
        assert_eq!(BlendMode::from_u8(1), BlendMode::Multiply);
        assert_eq!(BlendMode::from_u8(11), BlendMode::Exclusion);
        assert_eq!(BlendMode::from_u8(255), BlendMode::Normal);
    }

    #[test]
    fn test_layer_attribute_default() {
        let attr = LayerAttribute::default();
        assert_eq!(attr.filter_type, FilterType::None);
        assert_eq!(attr.blur_radius, 0.0);
        assert_eq!(attr.opacity, 1.0);
        assert_eq!(attr.blend_mode, BlendMode::Normal as u32);
    }

    #[test]
    fn test_layer_attribute_with_blur() {
        let attr = LayerAttribute::with_blur(5.0);
        assert_eq!(attr.filter_type, FilterType::Blur);
        assert_eq!(attr.blur_radius, 5.0);
        assert_eq!(attr.opacity, 1.0);
    }

    #[test]
    fn test_filter_blur() {
        let blur = Filter::blur(10.0);
        match blur {
            Filter::Blur { radius_x, radius_y } => {
                assert_eq!(radius_x, 10.0);
                assert_eq!(radius_y, 10.0);
            }
            _ => panic!("Expected Blur filter"),
        }
    }

    #[test]
    fn test_filter_drop_shadow_bounds() {
        let shadow = Filter::drop_shadow(5.0, 5.0, 10.0, 0xFF000000);
        let (l, t, r, b) = shadow.bounds_expansion();
        // With positive dx/dy, right and bottom expand more
        assert!(r > l);
        assert!(b > t);
    }

    #[test]
    fn test_filter_combined_bounds() {
        let blur = Filter::blur(5.0);
        let shadow = Filter::drop_shadow(3.0, 3.0, 8.0, 0xFF000000);
        let combined = Filter::combine(vec![blur, shadow]);
        let (l, t, r, b) = combined.bounds_expansion();
        assert!(l >= 5.0);
        assert!(t >= 5.0);
    }

    #[test]
    fn test_filter_registry() {
        let mut registry = FilterRegistry::new();
        let filter_id = FilterId::new(1);
        registry.define_filter(filter_id, Filter::blur(5.0));

        assert_eq!(registry.len(), 1);
        assert!(registry.has_filter(filter_id));

        let filter = registry.get_filter(filter_id);
        assert!(filter.is_some());
    }

    #[test]
    fn test_rect_contains() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert!(rect.contains((50.0, 50.0)));
        assert!(!rect.contains((150.0, 50.0)));
        assert!(!rect.contains((50.0, 150.0)));
    }

    #[test]
    fn test_rect_expand() {
        let rect = Rect::new(10.0, 10.0, 80.0, 80.0);
        let expanded = rect.expand(5.0, 5.0, 5.0, 5.0);
        assert_eq!(expanded.x, 5.0);
        assert_eq!(expanded.y, 5.0);
        assert_eq!(expanded.width, 90.0);
        assert_eq!(expanded.height, 90.0);
    }
}
