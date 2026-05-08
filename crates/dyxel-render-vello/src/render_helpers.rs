// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Render helper methods: frame setup, scene building, cache management, and diagnostics.

use crate::cache::CachedDraw;
use crate::coordinates::platform_correction;
use crate::{VelloBackend, NODE_COUNTER};
use kurbo::Vec2;
use vello::{Renderer, Scene};

impl VelloBackend {
    #[inline]
    pub(crate) fn clear_surface(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_view: &wgpu::TextureView,
    ) -> anyhow::Result<Option<vello::wgpu::SubmissionIndex>> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Clear Surface (Async Loading)"),
        });

        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), // Clear to black
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        let submission_index = queue.submit(Some(encoder.finish()));
        // Present is handled by the caller (old render_package) or GraphicsRuntime::end_frame (new path)

        Ok(Some(submission_index))
    }

    #[inline]
    pub(crate) fn build_main_scene(
        &self,
        root_id: Option<u32>,
        node_map: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
        scene: &mut Scene,
        cached_draws: &mut Vec<CachedDraw>,
        viewport_h: u32,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
    ) {
        if let Some(id) = root_id {
            // DEBUG: Reset node counter for this frame.
            NODE_COUNTER.store(0, std::sync::atomic::Ordering::SeqCst);

            let root_transform = platform_correction(viewport_h as f64);
            self.blur_state.with_scene_entries(
                |filter_pipeline, blurred_textures, blur_scene_frame| {
                    self.render_node_recursive_with_transform(
                        id,
                        node_map,
                        scene,
                        Vec2::ZERO,
                        root_transform,
                        device,
                        queue,
                        renderer,
                        filter_pipeline,
                        blurred_textures,
                        cached_draws,
                        false,
                        blur_scene_frame,
                    );
                },
            );
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub(crate) fn debug_save_pass1_scene_texture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_texture: &wgpu::Texture,
    ) {
        if !self.debug_frames_enabled() {
            return;
        }

        let frame_num = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let debug_dir = self.debug_output_dir();
        let path = debug_dir.join(format!("frame_{:06}_pass1_scene.png", frame_num % 1000));
        self.save_texture_to_png(device, queue, scene_texture, path.to_str().unwrap());

        // Debug: Sample pixels at blur card locations (expected to show purple background).
        log::debug!("[Debug] Sampling scene texture at blur card locations (expected purple bg)");
    }
}
