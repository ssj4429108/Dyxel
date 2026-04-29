// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! VelloDrawingBackend — RenderBackendV2 implementation for Vello + wgpu
//!
//! This is the drawing-only backend in the double-layer model.
//! It delegates scene rendering to the existing `VelloBackend` but does NOT
//! manage surface lifecycle, device creation, or presentation — that is
//! `WgpuRuntime`'s responsibility.

use dyxel_render_api::{
    BackendConfig, BackendFrameContext, DeviceHandle, GraphicsRuntime, LifecycleEvent,
    RenderBackend, RenderBackendV2, RenderFrameStats, RenderPackage, RuntimeKind,
};

/// Drawing-only backend for Vello + wgpu.
pub struct VelloDrawingBackend {
    vello_backend: super::VelloBackend,
    data_dir: String,
}

impl VelloDrawingBackend {
    pub fn new() -> Self {
        Self {
            vello_backend: super::VelloBackend::new(),
            data_dir: String::new(),
        }
    }

    pub fn with_data_dir(data_dir: String) -> Self {
        Self {
            vello_backend: super::VelloBackend::new(),
            data_dir,
        }
    }

    /// Access the underlying VelloBackend (transition helper for Phase 3A).
    pub fn vello_backend(&self) -> &super::VelloBackend {
        &self.vello_backend
    }
}

impl RenderBackendV2 for VelloDrawingBackend {
    fn initialize(
        &mut self,
        runtime: &mut dyn GraphicsRuntime,
    ) -> anyhow::Result<()> {
        let wgpu_runtime = runtime
            .as_any_mut()
            .downcast_mut::<super::runtime::WgpuRuntime>()
            .ok_or_else(|| anyhow::anyhow!("Runtime is not a WgpuRuntime"))?;

        let device = wgpu_runtime
            .device()
            .ok_or_else(|| anyhow::anyhow!("No wgpu device available"))?;
        let queue = wgpu_runtime
            .queue()
            .ok_or_else(|| anyhow::anyhow!("No wgpu queue available"))?;

        let device_handle = DeviceHandle::new(device);
        let queue_handle = dyxel_render_api::QueueHandle::new(queue);
        let config = BackendConfig {
            data_dir: self.data_dir.clone(),
        };

        self.vello_backend.init(device_handle, queue_handle, config)?;
        Ok(())
    }

    fn render(
        &mut self,
        frame: &mut dyn BackendFrameContext,
        package: &RenderPackage,
    ) -> anyhow::Result<RenderFrameStats> {
        if frame.runtime_kind() != RuntimeKind::Wgpu {
            return Err(anyhow::anyhow!(
                "VelloDrawingBackend: expected Wgpu runtime, got {:?}",
                frame.runtime_kind()
            ));
        }

        let frame = frame
            .as_any()
            .downcast_mut::<super::frame_context::WgpuFrameContext>()
            .ok_or_else(|| anyhow::anyhow!("Invalid frame context type"))?;

        let submission_index = if frame.render_to_offscreen {
            self.vello_backend.render_to_view(
                &frame.device,
                &frame.queue,
                &frame.view,
                frame.format,
                package,
            )?
        } else {
            self.vello_backend.render_with_surface_texture(
                &frame.device,
                &frame.queue,
                frame.surface_texture.as_ref().ok_or_else(|| anyhow::anyhow!("Missing surface texture in frame context"))?,
                frame.format,
                package,
            )?
        };
        frame.last_submission_index = submission_index;

        Ok(RenderFrameStats {
            cpu_time_ms: None,
            gpu_time_ms: None,
            backend_internal_stats: None,
        })
    }

    fn set_frame_timing(&self, pacer_wait_ms: f64, frame_interval_ms: f64) {
        self.vello_backend
            .set_frame_timing(pacer_wait_ms, frame_interval_ms);
    }

    fn set_frame_performance_stats(&self, stats: dyxel_perf::FramePerformanceStats) {
        self.vello_backend.set_frame_performance_stats(stats);
    }

    fn on_lifecycle_event(
        &self,
        event: LifecycleEvent,
    ) -> anyhow::Result<()> {
        self.vello_backend.on_lifecycle_event(event);
        Ok(())
    }

    fn enable_perf_overlay(&self) {
        self.vello_backend.enable_perf_overlay();
    }
}
