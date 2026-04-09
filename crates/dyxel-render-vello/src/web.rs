// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_render_api::{RenderContext, SurfaceState};
use vello::wgpu;

pub struct WebVelloSurfaceState {
    pub surface: vello::util::RenderSurface<'static>,
    pub blit_pipeline: wgpu::RenderPipeline,
    pub offscreen_texture: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
}

// Web doesn't need Send + Sync, but we keep them for API consistency
unsafe impl Send for WebVelloSurfaceState {}
unsafe impl Sync for WebVelloSurfaceState {}

impl SurfaceState for WebVelloSurfaceState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn resize(&mut self, context: &mut RenderContext, width: u32, height: u32) {
        // Downcast RenderContext to vello::util::RenderContext
        if let Some(v_ctx) = context.downcast_mut::<vello::util::RenderContext>() {
            v_ctx.resize_surface(&mut self.surface, width, height);
        }
    }
    fn width(&self) -> u32 {
        self.surface.config.width
    }
    fn height(&self) -> u32 {
        self.surface.config.height
    }
}
