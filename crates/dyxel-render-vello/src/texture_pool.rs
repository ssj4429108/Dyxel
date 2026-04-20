// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Texture Pool for efficient blur texture reuse
//!
//! Eliminates per-frame texture allocation by pooling textures based on size.
//! Supports Dual Kawase blur's 4-texture set (ds_half, ds_quarter, ping, pong).
//! All intermediate textures use Rgba16Float to prevent rounding errors.

use std::collections::HashMap;
use std::sync::Arc;

/// A pooled texture that automatically returns to the pool when dropped
pub struct PooledTexture {
    // Option wrappers allow safe move in Drop without unsafe zeroed()
    texture: Option<wgpu::Texture>,
    view: Option<wgpu::TextureView>,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
    // Internal channel to return texture to pool
    return_sender: Option<std::sync::mpsc::Sender<TextureReturn>>,
}

impl PooledTexture {
    /// Get reference to the texture
    pub fn texture(&self) -> &wgpu::Texture {
        self.texture.as_ref().unwrap()
    }

    /// Get reference to the texture view
    pub fn view(&self) -> &wgpu::TextureView {
        self.view.as_ref().unwrap()
    }
}

impl Drop for PooledTexture {
    fn drop(&mut self) {
        if let Some(sender) = self.return_sender.take() {
            // Send texture back to pool (ignore send failure if pool was dropped)
            if let (Some(texture), Some(view)) = (self.texture.take(), self.view.take()) {
                let _ = sender.send(TextureReturn {
                    size: self.size,
                    format: self.format,
                    texture,
                    view,
                });
            }
        }
    }
}

// Internal message for returning textures to pool
struct TextureReturn {
    size: (u32, u32),
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// A set of 4 textures for Dual Kawase blur
pub struct KawaseTextureSet {
    /// Downsample 1/2 resolution (half size of input)
    pub ds_half: PooledTexture,
    /// Downsample 1/8 resolution (1/8 size of input for reduced fill rate)
    pub ds_quarter: PooledTexture,
    /// Ping-pong buffer A (1/8 size)
    pub ping: PooledTexture,
    /// Ping-pong buffer B (1/8 size)
    pub pong: PooledTexture,
}

/// Pool configuration
#[derive(Clone, Copy, Debug)]
pub struct TexturePoolConfig {
    /// Maximum textures per (size, format) bucket
    pub max_per_bucket: usize,
    /// Maximum total pool size in bytes (approximate)
    pub max_total_bytes: usize,
}

impl Default for TexturePoolConfig {
    fn default() -> Self {
        Self {
            max_per_bucket: 16,
            max_total_bytes: 64 * 1024 * 1024, // 64MB
        }
    }
}

/// Texture pool for efficient reuse
pub struct TexturePool {
    device: Arc<wgpu::Device>,
    config: TexturePoolConfig,
    // Buckets indexed by (width/64, height/64, format) for efficient lookup
    buckets: HashMap<(u32, u32, u8), Vec<(wgpu::Texture, wgpu::TextureView)>>,
    // Return channel receiver
    return_receiver: std::sync::mpsc::Receiver<TextureReturn>,
    // Return channel sender (cloned for each PooledTexture)
    return_sender: std::sync::mpsc::Sender<TextureReturn>,
    // Track approximate memory usage
    current_bytes: usize,
}

// Format to index mapping
fn format_index(format: wgpu::TextureFormat) -> u8 {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => 0,
        wgpu::TextureFormat::Rgba16Float => 1,
        wgpu::TextureFormat::Bgra8Unorm => 2,
        _ => 255, // Unknown format
    }
}

// Calculate bucket key using exact dimensions to prevent texture size mismatches
// in blur pipelines where exact texel dimensions affect shader sampling.
fn bucket_key(width: u32, height: u32, format: wgpu::TextureFormat) -> (u32, u32, u8) {
    (width, height, format_index(format))
}

// Calculate texture size in bytes
fn texture_bytes(width: u32, height: u32, format: wgpu::TextureFormat) -> usize {
    let bytes_per_pixel = match format {
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm => 4,
        wgpu::TextureFormat::Rgba16Float => 8,
        _ => 4,
    };
    (width * height) as usize * bytes_per_pixel
}

impl TexturePool {
    /// Create a new texture pool
    pub fn new(device: Arc<wgpu::Device>, config: TexturePoolConfig) -> Self {
        let (return_sender, return_receiver) = std::sync::mpsc::channel();
        Self {
            device,
            config,
            buckets: HashMap::new(),
            return_receiver,
            return_sender,
            current_bytes: 0,
        }
    }

    /// Process returned textures (call at start of frame)
    pub fn collect_returns(&mut self) {
        while let Ok(ret) = self.return_receiver.try_recv() {
            // Use ACTUAL texture dimensions for the bucket key, not the requested size,
            // to ensure mismatched textures from the old 64px-bucket era are sorted correctly.
            let actual_w = ret.texture.width();
            let actual_h = ret.texture.height();
            let key = bucket_key(actual_w, actual_h, ret.format);
            let bucket = self.buckets.entry(key).or_default();

            // Only keep if under capacity
            if bucket.len() < self.config.max_per_bucket {
                bucket.push((ret.texture, ret.view));
            } else {
                // Drop excess (texture will be destroyed)
                self.current_bytes -= texture_bytes(actual_w, actual_h, ret.format);
            }
        }
    }

    /// Acquire a single texture from the pool
    pub fn acquire(
        &mut self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> PooledTexture {
        let key = bucket_key(width, height, format);

        // Try to get from bucket, validating actual dimensions match exactly.
        // Discard any mismatched leftovers from previous coarse-bucket behavior.
        if let Some(bucket) = self.buckets.get_mut(&key) {
            while let Some((texture, view)) = bucket.pop() {
                if texture.width() == width && texture.height() == height {
                    return PooledTexture {
                        texture: Some(texture),
                        view: Some(view),
                        size: (width, height),
                        format,
                        return_sender: Some(self.return_sender.clone()),
                    };
                }
                // Mismatch: destroy the stale texture
                self.current_bytes -= texture_bytes(texture.width(), texture.height(), format);
            }
        }

        // Create new texture
        let texture = self.create_texture(width, height, format);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.current_bytes += texture_bytes(width, height, format);

        PooledTexture {
            texture: Some(texture),
            view: Some(view),
            size: (width, height),
            format,
            return_sender: Some(self.return_sender.clone()),
        }
    }

    /// Acquire a complete Kawase texture set
    pub fn acquire_kawase_set(&mut self, full_width: u32, full_height: u32) -> KawaseTextureSet {
        // Calculate sizes
        let half_w = (full_width / 2).max(1);
        let half_h = (full_height / 2).max(1);
        // Use /8 to match internal KawaseTexturePool optimization (reduces fill rate to 1/64)
        let quarter_w = (full_width / 8).max(1);
        let quarter_h = (full_height / 8).max(1);

        KawaseTextureSet {
            ds_half: self.acquire(half_w, half_h, wgpu::TextureFormat::Rgba16Float),
            ds_quarter: self.acquire(quarter_w, quarter_h, wgpu::TextureFormat::Rgba16Float),
            ping: self.acquire(quarter_w, quarter_h, wgpu::TextureFormat::Rgba16Float),
            pong: self.acquire(quarter_w, quarter_h, wgpu::TextureFormat::Rgba16Float),
        }
    }

    /// Create a new texture with standard blur usage flags
    fn create_texture(
        &self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Pooled Blur Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Get current approximate memory usage
    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    /// Clear all pooled textures (call when low memory)
    pub fn clear(&mut self) {
        self.buckets.clear();
        self.current_bytes = 0;
    }
}

/// Opaque identifier for a pooled GPU texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureId(pub u32);

/// GPU texture pool compatible with the `RasterCache` / `OffscreenContext` APIs.
///
/// This is a thin wrapper around `TexturePool` that maps stable `TextureId`
/// handles to acquired `PooledTexture` entries.
pub struct GpuTexturePool {
    device: Arc<wgpu::Device>,
    inner: TexturePool,
    next_id: u32,
    textures: HashMap<TextureId, PooledTexture>,
}

impl GpuTexturePool {
    pub fn new(device: Arc<wgpu::Device>, config: TexturePoolConfig) -> Self {
        Self {
            device: device.clone(),
            inner: TexturePool::new(device, config),
            next_id: 1,
            textures: HashMap::new(),
        }
    }

    pub fn acquire(&mut self, width: u32, height: u32, format: wgpu::TextureFormat) -> TextureId {
        let id = TextureId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let tex = self.inner.acquire(width, height, format);
        self.textures.insert(id, tex);
        id
    }

    pub fn release(&mut self, id: TextureId) {
        self.textures.remove(&id);
    }

    pub fn get_texture(&self, id: TextureId) -> Option<&PooledTexture> {
        self.textures.get(&id)
    }

    pub fn collect_returns(&mut self) {
        self.inner.collect_returns();
    }
}

/// Thread-safe wrapper for TexturePool
///
/// Since wgpu::Texture is not Send + Sync, we use a separate pool per thread
/// or use the single-threaded approach with the main render thread.
pub struct SharedTexturePool {
    inner: std::sync::Mutex<TexturePool>,
}

impl SharedTexturePool {
    pub fn new(device: Arc<wgpu::Device>, config: TexturePoolConfig) -> Self {
        Self {
            inner: std::sync::Mutex::new(TexturePool::new(device, config)),
        }
    }

    pub fn collect_returns(&self) {
        if let Ok(mut pool) = self.inner.lock() {
            pool.collect_returns();
        }
    }

    pub fn acquire(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> PooledTexture {
        self.inner.lock().unwrap().acquire(width, height, format)
    }

    pub fn acquire_kawase_set(&self, full_width: u32, full_height: u32) -> KawaseTextureSet {
        self.inner
            .lock()
            .unwrap()
            .acquire_kawase_set(full_width, full_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bucket_key() {
        assert_eq!(
            bucket_key(64, 64, wgpu::TextureFormat::Rgba8Unorm),
            (64, 64, 0)
        );
        assert_eq!(
            bucket_key(65, 128, wgpu::TextureFormat::Rgba16Float),
            (65, 128, 1)
        );
        assert_eq!(
            bucket_key(256, 256, wgpu::TextureFormat::Bgra8Unorm),
            (256, 256, 2)
        );
    }
}
