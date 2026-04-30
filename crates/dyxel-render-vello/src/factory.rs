// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! VelloGraphicsFactory — GraphicsRuntimeFactory for the Vello + wgpu backend.

use dyxel_render_api::{BackendCapabilities, GraphicsRuntimeFactory, RenderBackendV2};

/// Factory for creating Vello + wgpu runtime/backend pairs.
pub struct VelloGraphicsFactory;

impl VelloGraphicsFactory {
    pub fn new() -> Self {
        Self
    }
}

impl GraphicsRuntimeFactory for VelloGraphicsFactory {
    fn backend_name(&self) -> &'static str {
        "vello"
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            perf_overlay: true,
            gpu_timing: true,
            renderer_warmup: true,
            main_thread_surface_creation: true,
            main_thread_rendering: false,
            explicit_present: true,
        }
    }

    fn create_runtime(&self) -> anyhow::Result<Box<dyn dyxel_render_api::GraphicsRuntime>> {
        let runtime = super::runtime::WgpuRuntime::new();
        Ok(Box::new(runtime))
    }

    fn create_backend(&self) -> anyhow::Result<Box<dyn RenderBackendV2>> {
        let backend = super::backend::VelloDrawingBackend::new();
        Ok(Box::new(backend))
    }
}

impl Default for VelloGraphicsFactory {
    fn default() -> Self {
        Self::new()
    }
}
