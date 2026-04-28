// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! WgpuFrameContext — BackendFrameContext implementation for Vello + wgpu
//!
//! Created by WgpuRuntime::begin_frame(), consumed by VelloBackend::render()
//! and WgpuRuntime::end_frame().

use dyxel_render_api::{BackendFrameContext, RuntimeKind};

/// Per-frame context for wgpu-backed rendering.
///
/// Holds the surface texture, its view, and device/queue references
/// needed by VelloBackend for this frame.
pub struct WgpuFrameContext {
    #[allow(dead_code)]
    pub(crate) surface_id: dyxel_render_api::RuntimeSurfaceId,
    pub(crate) surface_texture: Option<vello::wgpu::SurfaceTexture>,
    /// Texture kept alive when rendering into an offscreen target first.
    ///
    /// On macOS/Fifo this lets begin_frame avoid `get_current_texture()`; the
    /// runtime acquires the surface only in end_frame and blits this texture.
    pub(crate) offscreen_texture: Option<vello::wgpu::Texture>,
    #[allow(dead_code)]
    pub(crate) view: vello::wgpu::TextureView,
    pub(crate) render_to_offscreen: bool,
    pub(crate) device: vello::wgpu::Device,
    pub(crate) queue: vello::wgpu::Queue,
    pub(crate) format: vello::wgpu::TextureFormat,
    #[allow(dead_code)]
    pub(crate) width: u32,
    #[allow(dead_code)]
    pub(crate) height: u32,
    /// Time spent in `get_current_texture()` (surface acquisition / implicit sync wait).
    #[allow(dead_code)]
    pub(crate) acquire_ms: f64,
    /// Time spent in `present()` (GPU completion wait + VBlank block).
    pub(crate) present_ms: f64,
}

impl BackendFrameContext for WgpuFrameContext {
    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn runtime_kind(&self) -> RuntimeKind {
        RuntimeKind::Wgpu
    }
}
