// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Legacy direct `RenderBackend` compatibility implementation.
//!
//! The active renderer path uses `GraphicsRuntime` + `RenderBackendV2` via
//! `backend::VelloDrawingBackend`. This module keeps the older direct backend
//! API wired without leaving surface lifecycle code mixed into `lib.rs`.

#[cfg(target_os = "android")]
use crate::android;
#[cfg(target_os = "macos")]
use crate::mac;
#[cfg(target_arch = "wasm32")]
use crate::web;
use crate::{cold_start, filter_pipeline, state, texture_pool, VelloBackend};
use dyxel_render_api::{
    BackendConfig, DeviceHandle, LifecycleEvent, QueueHandle, RenderBackend, RenderContext,
    RenderResult, SurfaceHandle, SurfaceState, SurfaceTargetHandle,
};
use std::sync::Arc;
use vello::wgpu;

impl RenderBackend for VelloBackend {
    fn init(
        &self,
        device: DeviceHandle,
        _queue: QueueHandle,
        config: BackendConfig,
    ) -> RenderResult {
        let init_start = std::time::Instant::now();

        #[cfg(target_os = "android")]
        log::info!("[Android-Perf] VelloBackend::init started - Performance monitoring enabled");

        // Convert DeviceHandle to wgpu::Device reference
        let device = unsafe { &*device.as_ptr::<wgpu::Device>() };

        let (blit_shader, blit_bl, sampler) = state::BlitState::create_resources(device);

        let (cache_path, pipeline_cache, cache_stage) =
            cold_start::load_pipeline_cache(device, &config.data_dir);

        self.blit_state.set_resources(blit_shader, blit_bl, sampler);
        self.renderer_state.store_cache_info(
            cache_path.clone(),
            pipeline_cache.clone(),
            cache_stage,
        );

        // Prewarm blit pipeline
        self.prewarm_pipelines(device, wgpu::TextureFormat::Rgba8Unorm);

        // Initialize filter pipeline for blur effects
        let device_arc = std::sync::Arc::new(device.clone());
        let queue_arc = std::sync::Arc::new(unsafe { &*_queue.as_ptr::<wgpu::Queue>() }.clone());
        match filter_pipeline::FilterPipeline::new(device_arc, queue_arc) {
            Ok(pipeline) => {
                self.blur_state.set_filter_pipeline(pipeline);
                log::debug!("[Blur] Filter pipeline initialized successfully");
            }
            Err(e) => {
                log::warn!("[Blur] Failed to initialize filter pipeline: {}", e);
                // Continue without blur support
            }
        }

        // Note: Blur composite pipeline is created lazily on first use
        // with the correct surface format to avoid format mismatch

        // Initialize texture pool for efficient blur texture reuse
        {
            let device_arc = Arc::new(device.clone());
            let pool = texture_pool::SharedTexturePool::new(
                device_arc.clone(),
                texture_pool::TexturePoolConfig::default(),
            );
            self.blur_state.set_texture_pool(pool);
            let gpu_pool = texture_pool::GpuTexturePool::new(
                device_arc,
                texture_pool::TexturePoolConfig::default(),
            );
            self.raster_cache_state.set_gpu_texture_pool(gpu_pool);
            log::info!("[TexturePool] Initialized blur texture pool");
        }

        // Raster cache initialization has moved to Runtime.

        // Store info for deferred renderer initialization (includes cache stage)
        self.renderer_state
            .store_deferred_init_info(cache_path, pipeline_cache, cache_stage);

        // Eagerly start renderer initialization in background so first frame isn't black
        let queue_ref = unsafe { &*_queue.as_ptr::<wgpu::Queue>() };
        self.ensure_renderer_initialized_async(device, queue_ref);

        // Initialize memory optimizer
        self.renderer_state.initialize_memory_optimizer();

        log::info!(
            "[Perf] VelloBackend::init: Total time {:?} (Renderer deferred)",
            init_start.elapsed()
        );
        Ok(())
    }

    fn create_surface_state(
        &self,
        context: &mut RenderContext,
        target: Option<SurfaceTargetHandle>,
        surface: Option<SurfaceHandle>,
        _surface_ptr: u64,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Box<dyn SurfaceState>> {
        log::info!(
            "VelloBackend: create_surface_state START - size: {}x{}, has_precreated_surface: {}",
            width,
            height,
            surface.is_some()
        );

        // Downcast RenderContext to vello::util::RenderContext
        let v_ctx = context
            .downcast_mut::<vello::util::RenderContext>()
            .ok_or_else(|| anyhow::anyhow!("RenderContext is not a Vello RenderContext"))?;

        // Select present mode
        #[cfg(target_os = "android")]
        let present_mode = {
            log::info!("VelloBackend: Using Mailbox mode (low latency, VSync-like but faster)");
            wgpu::PresentMode::Mailbox
        };

        #[cfg(not(target_os = "android"))]
        let present_mode = {
            log::info!("VelloBackend: Using Immediate mode (VSync disabled)");
            wgpu::PresentMode::Immediate
        };

        let v_surface = if let Some(s) = surface {
            log::info!(
                "VelloBackend: Using pre-created surface (present_mode: {:?})",
                present_mode
            );
            let wgpu_surface = s
                .into_inner::<wgpu::Surface<'static>>()
                .ok_or_else(|| anyhow::anyhow!("SurfaceHandle is not a wgpu::Surface"))?;
            pollster::block_on(v_ctx.create_render_surface(
                wgpu_surface,
                width,
                height,
                present_mode,
            ))
            .map_err(|e| anyhow::anyhow!("Failed to create render surface: {:?}", e))?
        } else if let Some(t) = target {
            log::info!(
                "VelloBackend: Creating surface from target (present_mode: {:?})",
                present_mode
            );
            let wgpu_target = t
                .into_inner::<wgpu::SurfaceTarget<'static>>()
                .ok_or_else(|| {
                    anyhow::anyhow!("SurfaceTargetHandle is not a wgpu::SurfaceTarget")
                })?;
            pollster::block_on(v_ctx.create_surface(wgpu_target, width, height, present_mode))
                .map_err(|e| anyhow::anyhow!("Failed to create surface: {:?}", e))?
        } else {
            return Err(anyhow::anyhow!("Either target or surface must be provided"));
        };

        log::info!(
            "VelloBackend: Surface created, format: {:?}, dev_id: {}",
            v_surface.config.format,
            v_surface.dev_id
        );

        let device = &v_ctx.devices[v_surface.dev_id].device;
        let (blit_p, children_blit_p) = self.renderer_state.with_pipeline_cache(|pipeline_cache| {
            self.blit_state.create_surface_pipelines(
                device,
                v_surface.config.format,
                pipeline_cache,
            )
        });

        log::info!("VelloBackend: Blit pipeline created successfully");
        self.blit_state
            .set_surface_pipelines(blit_p, children_blit_p);

        #[cfg(target_os = "macos")]
        {
            log::info!("VelloBackend: Creating MacVelloSurfaceState");
            return Ok(Box::new(mac::MacVelloSurfaceState { surface: v_surface }));
        }

        #[cfg(target_os = "android")]
        {
            log::info!("VelloBackend: Creating AndroidVelloSurfaceState");
            return Ok(Box::new(android::AndroidVelloSurfaceState {
                surface: v_surface,
            }));
        }

        #[cfg(target_arch = "wasm32")]
        {
            log::info!("VelloBackend: Creating WebVelloSurfaceState");
            return Ok(Box::new(web::WebVelloSurfaceState { surface: v_surface }));
        }

        #[cfg(all(
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_arch = "wasm32")
        ))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn set_frame_timing(&self, pacer_wait_ms: f64, frame_interval_ms: f64) {
        self.diagnostics_state
            .set_frame_timing(pacer_wait_ms, frame_interval_ms);
    }

    fn set_frame_performance_stats(&self, stats: dyxel_perf::FramePerformanceStats) {
        self.diagnostics_state.set_frame_performance_stats(stats);
    }

    fn render_package(
        &self,
        device: DeviceHandle,
        queue: QueueHandle,
        surface: &mut dyn SurfaceState,
        package: &dyxel_render_api::RenderPackage,
    ) -> RenderResult {
        let device = unsafe { &*device.as_ptr::<wgpu::Device>() };
        let queue = unsafe { &*queue.as_ptr::<wgpu::Queue>() };

        #[cfg(target_os = "macos")]
        let render_surface = &surface
            .as_any_mut()
            .downcast_mut::<mac::MacVelloSurfaceState>()
            .ok_or_else(|| anyhow::anyhow!("Invalid surface state (not MacVelloSurfaceState)"))?
            .surface;

        #[cfg(target_os = "android")]
        let render_surface = &surface
            .as_any_mut()
            .downcast_mut::<android::AndroidVelloSurfaceState>()
            .ok_or_else(|| anyhow::anyhow!("Invalid surface state (not AndroidVelloSurfaceState)"))?
            .surface;

        #[cfg(target_arch = "wasm32")]
        let render_surface = &surface
            .as_any_mut()
            .downcast_mut::<web::WebVelloSurfaceState>()
            .ok_or_else(|| anyhow::anyhow!("Invalid surface state (not WebVelloSurfaceState)"))?
            .surface;

        #[cfg(any(target_os = "macos", target_os = "android", target_arch = "wasm32"))]
        {
            let st = render_surface
                .surface
                .get_current_texture()
                .map_err(|e| anyhow::anyhow!("Failed to get current texture: {:?}", e))?;
            let target_view = st.texture.create_view(&Default::default());
            let _ = self.render_internal_impl(
                device,
                queue,
                &target_view,
                render_surface.format,
                package,
            )?;
            st.present();
            Ok(())
        }

        #[cfg(all(
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_arch = "wasm32")
        ))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn on_lifecycle_event(&self, event: LifecycleEvent) {
        match event {
            LifecycleEvent::FirstFrameDone | LifecycleEvent::Shutdown => {
                self.save_cache();
            }
            _ => {}
        }
    }

    fn sync_gpu(&self, _device: DeviceHandle, queue: QueueHandle) {
        let queue = unsafe { &*queue.as_ptr::<wgpu::Queue>() };

        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        queue.on_submitted_work_done(move || {
            let _ = tx.send(());
        });

        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(_) => log::info!("VelloBackend: sync_gpu completed successfully"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                log::warn!("VelloBackend: sync_gpu timed out, GPU may be unresponsive");
            }
            Err(e) => log::error!("VelloBackend: sync_gpu error: {:?}", e),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
