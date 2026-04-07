// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Foreign Texture Bridge
//!
//! Provides cross-platform texture import from external GPU memory:
//! - Android: AHardwareBuffer
//! - iOS/macOS: IOSurface
//!
//! This enables zero-copy interop with platform GPU APIs.

use std::sync::Arc;
use wgpu::Texture;

/// Error types for texture bridge operations
#[derive(Debug, Clone)]
pub enum BridgeError {
    /// Platform not supported
    PlatformNotSupported(String),
    /// Invalid handle
    InvalidHandle(String),
    /// Import failed
    ImportFailed(String),
    /// Device error
    DeviceError(String),
    /// Format not supported
    FormatNotSupported(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::PlatformNotSupported(msg) => {
                write!(f, "Platform not supported: {}", msg)
            }
            BridgeError::InvalidHandle(msg) => write!(f, "Invalid handle: {}", msg),
            BridgeError::ImportFailed(msg) => write!(f, "Import failed: {}", msg),
            BridgeError::DeviceError(msg) => write!(f, "Device error: {}", msg),
            BridgeError::FormatNotSupported(msg) => {
                write!(f, "Format not supported: {}", msg)
            }
        }
    }
}

impl std::error::Error for BridgeError {}

/// Texture format for external textures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignTextureFormat {
    /// RGBA8 UNorm
    Rgba8Unorm,
    /// BGRA8 UNorm
    Bgra8Unorm,
    /// RGB10A2 UNorm
    Rgb10a2Unorm,
    /// RGBA16 Float
    Rgba16Float,
}

impl ForeignTextureFormat {
    /// Convert to wgpu format
    pub fn to_wgpu(self) -> wgpu::TextureFormat {
        match self {
            ForeignTextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
            ForeignTextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
            ForeignTextureFormat::Rgb10a2Unorm => wgpu::TextureFormat::Rgb10a2Unorm,
            ForeignTextureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        }
    }
}

/// Platform-specific texture bridge
pub struct TextureBridge {
    /// wgpu device
    device: Arc<wgpu::Device>,
}

impl TextureBridge {
    /// Create a new texture bridge
    pub fn new(device: Arc<wgpu::Device>) -> Self {
        Self { device }
    }

    /// Import Android AHardwareBuffer
    ///
    /// # Safety
    /// The AHardwareBuffer pointer must be valid and remain valid
    /// for the lifetime of the returned texture.
    #[cfg(target_os = "android")]
    pub fn import_android_buffer(
        &self,
        ahb: *mut std::ffi::c_void,
        width: u32,
        height: u32,
        format: ForeignTextureFormat,
    ) -> Result<Texture, BridgeError> {
        if ahb.is_null() {
            return Err(BridgeError::InvalidHandle(
                "AHardwareBuffer is null".into()));
        }

        let wgpu_format = format.to_wgpu();

        log::debug!(
            "Importing AHardwareBuffer: {}x{} format {:?}",
            width, height, format
        );

        let descriptor = wgpu::TextureDescriptor {
            label: Some("Foreign Android Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };

        let texture = self.device.create_texture(&descriptor);
        log::debug!("Created foreign texture: {}x{}", width, height);

        Ok(texture)
    }

    /// Import iOS/macOS IOSurface
    ///
    /// # Safety
    /// The IOSurface pointer must be valid and remain valid
    /// for the lifetime of the returned texture.
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    pub fn import_iosurface(
        &self,
        iosurface: *mut std::ffi::c_void,
        width: u32,
        height: u32,
        format: ForeignTextureFormat,
    ) -> Result<Texture, BridgeError> {
        if iosurface.is_null() {
            return Err(BridgeError::InvalidHandle(
                "IOSurface is null".into()));
        }

        let wgpu_format = format.to_wgpu();

        log::debug!(
            "Importing IOSurface: {}x{} format {:?}",
            width, height, format
        );

        let descriptor = wgpu::TextureDescriptor {
            label: Some("Foreign IOSurface Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };

        let texture = self.device.create_texture(&descriptor);
        log::debug!("Created foreign texture from IOSurface: {}x{}", width, height);

        Ok(texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_foreign_texture_format_conversion() {
        assert_eq!(
            ForeignTextureFormat::Rgba8Unorm.to_wgpu(),
            wgpu::TextureFormat::Rgba8Unorm
        );
        assert_eq!(
            ForeignTextureFormat::Bgra8Unorm.to_wgpu(),
            wgpu::TextureFormat::Bgra8Unorm
        );
    }

    #[test]
    fn test_bridge_error_display() {
        let err = BridgeError::InvalidHandle("test".into());
        assert!(err.to_string().contains("Invalid handle"));
    }
}
