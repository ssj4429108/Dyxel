// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_render_api::{SurfaceState, RenderContext};
use vello::wgpu;

pub struct AndroidVelloSurfaceState {
    pub surface: vello::util::RenderSurface<'static>,
    pub blit_pipeline: wgpu::RenderPipeline,
    pub offscreen_texture: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
}

unsafe impl Send for AndroidVelloSurfaceState {}
unsafe impl Sync for AndroidVelloSurfaceState {}

impl SurfaceState for AndroidVelloSurfaceState {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn resize(&mut self, context: &mut RenderContext, width: u32, height: u32) {
        // Downcast RenderContext to vello::util::RenderContext
        if let Some(v_ctx) = context.downcast_mut::<vello::util::RenderContext>() {
            v_ctx.resize_surface(&mut self.surface, width, height);
        }
    }
    fn width(&self) -> u32 { self.surface.config.width }
    fn height(&self) -> u32 { self.surface.config.height }
}
