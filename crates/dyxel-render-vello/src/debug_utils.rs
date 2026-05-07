// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Debug utilities: texture saving, frame capture config.

use crate::VelloBackend;

impl VelloBackend {
    /// Save texture to PNG file for debugging
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn save_texture_to_png(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        path: &str,
    ) {
        // Debug save disabled
        let size = texture.size();
        let format = texture.format();

        // wgpu requires bytes_per_row to be a multiple of 256
        let bytes_per_pixel = match format {
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm => 4,
            _ => 4,
        };
        let bytes_per_row_unaligned = size.width * bytes_per_pixel;
        let bytes_per_row = ((bytes_per_row_unaligned + 255) / 256) * 256;
        let buffer_size = (bytes_per_row * size.height) as u64;

        // Create buffer to read back
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Readback Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // Copy texture to buffer
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        queue.submit(Some(encoder.finish()));

        // Map and save
        let buffer_slice = readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        // Poll device until mapping completes
        while rx.try_recv().is_err() {
            let _ = device.poll(wgpu::PollType::Poll);
        }

        {
            let data = buffer_slice.get_mapped_range();
            let rgba_data: &[u8] = &data;

            // Copy row by row to handle alignment
            let mut img_data = Vec::with_capacity((size.width * size.height * 3) as usize);
            for row in 0..size.height {
                let row_start = (row * bytes_per_row) as usize;
                for col in 0..size.width {
                    let pixel_offset = row_start + (col * bytes_per_pixel) as usize;
                    if pixel_offset + 2 < rgba_data.len() {
                        // Handle BGRA vs RGBA
                        if format == wgpu::TextureFormat::Bgra8Unorm {
                            img_data.push(rgba_data[pixel_offset + 2]); // R (from B)
                            img_data.push(rgba_data[pixel_offset + 1]); // G
                            img_data.push(rgba_data[pixel_offset]); // B (from R)
                        } else {
                            img_data.push(rgba_data[pixel_offset]); // R
                            img_data.push(rgba_data[pixel_offset + 1]); // G
                            img_data.push(rgba_data[pixel_offset + 2]); // B
                        }
                    }
                }
            }

            let img = image::RgbImage::from_raw(size.width, size.height, img_data);
            if let Some(img) = img {
                if let Err(e) = img.save(path) {
                    log::warn!("Failed to save debug image {}: {}", path, e);
                }
            } else {
                log::warn!("Failed to create image from raw data");
            }
        }
        readback_buffer.unmap();
    }

    /// Check if debug frame saving is enabled
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn debug_frames_enabled(&self) -> bool {
        false // Disabled: debug frame saving is off by default
    }

    /// Get debug output directory
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn debug_output_dir(&self) -> std::path::PathBuf {
        let dir = std::env::var("DYXEL_DEBUG_DIR").unwrap_or_else(|_| "debug_frames".to_string());
        let path = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&path).ok();
        path
    }

    /// Create debug capture texture and render target view.
    /// Returns `(capture_texture, debug_frame_num, render_target_view)`.
    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub(crate) fn create_debug_capture(
        &self,
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        target_view: &wgpu::TextureView,
        w: u32,
        h: u32,
    ) -> (Option<wgpu::Texture>, Option<u64>, wgpu::TextureView) {
        let debug_frame_num = if self.debug_frames_enabled() {
            let frame_num = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Some(frame_num % 1000)
        } else {
            None
        };

        let capture_texture = if self.debug_frames_enabled() && debug_frame_num.is_some() {
            let capture_tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Capture Texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: surface_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            Some(capture_tex)
        } else {
            None
        };

        let render_target_view = if let Some(ref capture_tex) = capture_texture {
            capture_tex.create_view(&Default::default())
        } else {
            target_view.clone()
        };

        (capture_texture, debug_frame_num, render_target_view)
    }
}
