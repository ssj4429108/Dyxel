// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Device Information & Logical Pixel System
//!
//! Similar to Flutter's device pixel ratio system:
//! - Logical pixels (LP) are the default unit in RSX
//! - Physical pixels = LP × device_pixel_ratio
//!
//! ## Size Units
//!
//! | RSX Syntax | Unit | Example |
//! |------------|------|---------|
//! | `100.0` | Logical pixels (LP) | `width: 100.0` |
//! | `px(100)` | Physical pixels | `width: px(100)` |
//! | `"100%"` | Percentage | `width: "100%"` |
//! | `"auto"` | Auto | `width: "auto"` |
//! | `100.lp()` | Explicit LP | `width: 100.lp()` |

/// Device information from Host
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceInfo {
    /// Device pixel ratio (physical pixels per logical pixel)
    /// iPhone 14 = 3.0, Android mdpi = 1.0, xhdpi = 2.0
    pub device_pixel_ratio: f32,
    /// Font scale factor (system accessibility setting)
    pub text_scale_factor: f32,
    /// Screen width in logical pixels
    pub screen_width_lp: f32,
    /// Screen height in logical pixels
    pub screen_height_lp: f32,
    /// Safe area top inset (notch/status bar)
    pub safe_area_top: f32,
    /// Safe area bottom inset (home indicator)
    pub safe_area_bottom: f32,
    /// Platform type (0=Android, 1=iOS, 2=macOS, 3=Web, 4=Other)
    pub platform: u32,
    /// Reserved for future use
    pub _padding: [f32; 3],
}

impl DeviceInfo {
    /// Convert logical pixels to physical pixels
    pub fn lp_to_px(&self, lp: f32) -> f32 {
        lp * self.device_pixel_ratio
    }

    /// Convert physical pixels to logical pixels
    pub fn px_to_lp(&self, px: f32) -> f32 {
        px / self.device_pixel_ratio
    }

    /// Apply font scale to logical font size
    pub fn scale_font(&self, lp: f32) -> f32 {
        lp * self.text_scale_factor
    }

    /// Get safe area insets
    pub fn safe_insets(&self) -> (f32, f32, f32, f32) {
        (self.safe_area_top, 0.0, self.safe_area_bottom, 0.0)
    }
}

/// Size unit for layout - supports LP, PX, Percent, Auto
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeUnit {
    /// Logical pixels (default)
    Lp(f32),
    /// Physical pixels (absolute)
    Px(f32),
    /// Percentage of parent (0.0 - 100.0)
    Percent(f32),
    /// Automatic sizing
    Auto,
}

impl SizeUnit {
    /// Convert to logical pixels
    pub fn to_lp(&self, device_info: &DeviceInfo) -> f32 {
        match self {
            SizeUnit::Lp(v) => *v,
            SizeUnit::Px(v) => device_info.px_to_lp(*v),
            SizeUnit::Percent(_) => 0.0, // Requires parent size
            SizeUnit::Auto => 0.0,
        }
    }

    /// Convert to physical pixels
    pub fn to_px(&self, device_info: &DeviceInfo) -> f32 {
        match self {
            SizeUnit::Lp(v) => device_info.lp_to_px(*v),
            SizeUnit::Px(v) => *v,
            SizeUnit::Percent(_) => 0.0, // Requires parent size
            SizeUnit::Auto => 0.0,
        }
    }

    /// Check if auto
    pub fn is_auto(&self) -> bool {
        matches!(self, SizeUnit::Auto)
    }
}

impl Default for SizeUnit {
    fn default() -> Self {
        SizeUnit::Auto
    }
}

impl From<f32> for SizeUnit {
    fn from(f: f32) -> Self {
        SizeUnit::Lp(f)
    }
}

impl From<i32> for SizeUnit {
    fn from(i: i32) -> Self {
        SizeUnit::Lp(i as f32)
    }
}

impl From<&str> for SizeUnit {
    fn from(s: &str) -> Self {
        if s == "auto" {
            SizeUnit::Auto
        } else if s.ends_with('%') {
            SizeUnit::Percent(s[..s.len()-1].parse().unwrap_or(0.0))
        } else {
            SizeUnit::Lp(s.parse().unwrap_or(0.0))
        }
    }
}

/// Font size unit - supports LP and absolute
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontSizeUnit {
    /// Logical pixels (scales with textScaleFactor)
    Lp(f32),
    /// Physical pixels (absolute, no scaling)
    Px(f32),
}

impl FontSizeUnit {
    /// Get final font size in logical pixels
    pub fn to_lp(&self, device_info: &DeviceInfo) -> f32 {
        match self {
            FontSizeUnit::Lp(v) => device_info.scale_font(*v),
            FontSizeUnit::Px(v) => device_info.px_to_lp(*v),
        }
    }

    /// Get final font size in physical pixels
    pub fn to_px(&self, device_info: &DeviceInfo) -> f32 {
        match self {
            FontSizeUnit::Lp(v) => device_info.lp_to_px(device_info.scale_font(*v)),
            FontSizeUnit::Px(v) => *v,
        }
    }
}

impl Default for FontSizeUnit {
    fn default() -> Self {
        FontSizeUnit::Lp(14.0)
    }
}

impl From<f32> for FontSizeUnit {
    fn from(f: f32) -> Self {
        FontSizeUnit::Lp(f)
    }
}

/// Helper trait for RSX px() syntax
pub trait PxExt {
    fn px(self) -> SizeUnit;
}

impl PxExt for f32 {
    fn px(self) -> SizeUnit {
        SizeUnit::Px(self)
    }
}

impl PxExt for i32 {
    fn px(self) -> SizeUnit {
        SizeUnit::Px(self as f32)
    }
}

/// Helper function for px() syntax
pub fn px<T: PxExt>(v: T) -> SizeUnit {
    v.px()
}

/// Helper trait for explicit LP syntax
pub trait LpExt {
    fn lp(self) -> SizeUnit;
}

impl LpExt for f32 {
    fn lp(self) -> SizeUnit {
        SizeUnit::Lp(self)
    }
}

impl LpExt for i32 {
    fn lp(self) -> SizeUnit {
        SizeUnit::Lp(self as f32)
    }
}

/// Helper function for lp() syntax
pub fn lp<T: LpExt>(v: T) -> SizeUnit {
    v.lp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lp_px_conversion() {
        let device = DeviceInfo {
            device_pixel_ratio: 2.0,
            text_scale_factor: 1.0,
            screen_width_lp: 375.0,
            screen_height_lp: 812.0,
            safe_area_top: 44.0,
            safe_area_bottom: 34.0,
            platform: 0,
            _padding: [0.0; 3],
        };

        // LP to PX
        assert_eq!(device.lp_to_px(100.0), 200.0);
        
        // PX to LP
        assert_eq!(device.px_to_lp(200.0), 100.0);
        
        // Font scaling
        assert_eq!(device.scale_font(16.0), 16.0);
    }

    #[test]
    fn test_size_unit() {
        let device = DeviceInfo {
            device_pixel_ratio: 3.0,
            text_scale_factor: 1.0,
            ..Default::default()
        };

        // LP input
        assert_eq!(SizeUnit::Lp(100.0).to_px(&device), 300.0);
        
        // PX input
        assert_eq!(SizeUnit::Px(300.0).to_lp(&device), 100.0);
    }

    #[test]
    fn test_px_ext() {
        assert!(matches!(100f32.px(), SizeUnit::Px(100.0)));
        assert!(matches!(100i32.px(), SizeUnit::Px(100.0)));
        assert!(matches!(100f32.lp(), SizeUnit::Lp(100.0)));
    }
}
