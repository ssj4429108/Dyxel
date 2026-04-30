// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_render_api::{BackendCapabilities, GraphicsRuntimeFactory, RenderBackendV2};

/// Graphics factory for the experimental Impeller backend.
pub struct ImpellerGraphicsFactory;

impl ImpellerGraphicsFactory {
    pub fn new() -> Self {
        Self
    }
}

impl GraphicsRuntimeFactory for ImpellerGraphicsFactory {
    fn backend_name(&self) -> &'static str {
        "impeller"
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            perf_overlay: false,
            gpu_timing: false,
            renderer_warmup: false,
            main_thread_surface_creation: true,
            main_thread_rendering: false,
            explicit_present: true,
        }
    }

    fn create_runtime(&self) -> anyhow::Result<Box<dyn dyxel_render_api::GraphicsRuntime>> {
        Ok(Box::new(super::runtime::ImpellerRuntime::new()))
    }

    fn create_backend(&self) -> anyhow::Result<Box<dyn RenderBackendV2>> {
        Ok(Box::new(super::backend::ImpellerDrawingBackend::new()))
    }
}

impl Default for ImpellerGraphicsFactory {
    fn default() -> Self {
        Self::new()
    }
}
