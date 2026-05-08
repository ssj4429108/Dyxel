// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! WgpuFrameContext — BackendFrameContext implementation for Vello + wgpu
//!
//! Created by WgpuRuntime::begin_frame(), consumed by VelloBackend::render()
//! and WgpuRuntime::end_frame().

use dyxel_render_api::{BackendFrameContext, RuntimeKind};
#[cfg(target_os = "android")]
use std::sync::{Arc, Mutex};

#[cfg(target_os = "android")]
pub(crate) struct DetachedBlitState {
    layout: Option<vello::wgpu::BindGroupLayout>,
    shader: Option<vello::wgpu::ShaderModule>,
    pipeline: Option<vello::wgpu::RenderPipeline>,
    pipeline_format: Option<vello::wgpu::TextureFormat>,
    sampler: Option<vello::wgpu::Sampler>,
}

#[cfg(target_os = "android")]
impl DetachedBlitState {
    pub(crate) fn new() -> Self {
        Self {
            layout: None,
            shader: None,
            pipeline: None,
            pipeline_format: None,
            sampler: None,
        }
    }

    fn ensure_pipeline(
        &mut self,
        device: &vello::wgpu::Device,
        format: vello::wgpu::TextureFormat,
    ) {
        if self.pipeline.is_some() && self.pipeline_format == Some(format) {
            return;
        }

        let shader = self.shader.get_or_insert_with(|| {
            device.create_shader_module(vello::wgpu::ShaderModuleDescriptor {
                label: Some("Detached Presenter Blit Shader"),
                source: vello::wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
            })
        });

        let layout = self.layout.get_or_insert_with(|| {
            device.create_bind_group_layout(&vello::wgpu::BindGroupLayoutDescriptor {
                label: Some("Detached Presenter Blit BGL"),
                entries: &[
                    vello::wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: vello::wgpu::ShaderStages::FRAGMENT,
                        ty: vello::wgpu::BindingType::Texture {
                            sample_type: vello::wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: vello::wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    vello::wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: vello::wgpu::ShaderStages::FRAGMENT,
                        ty: vello::wgpu::BindingType::Sampler(
                            vello::wgpu::SamplerBindingType::Filtering,
                        ),
                        count: None,
                    },
                ],
            })
        });

        self.sampler.get_or_insert_with(|| {
            device.create_sampler(&vello::wgpu::SamplerDescriptor {
                label: Some("Detached Presenter Blit Sampler"),
                mag_filter: vello::wgpu::FilterMode::Linear,
                min_filter: vello::wgpu::FilterMode::Linear,
                ..Default::default()
            })
        });

        let pipeline_layout =
            device.create_pipeline_layout(&vello::wgpu::PipelineLayoutDescriptor {
                label: Some("Detached Presenter Blit Pipeline Layout"),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            });
        self.pipeline = Some(device.create_render_pipeline(
            &vello::wgpu::RenderPipelineDescriptor {
                label: Some("Detached Presenter Blit Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: vello::wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(vello::wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(vello::wgpu::ColorTargetState {
                        format,
                        blend: Some(vello::wgpu::BlendState::REPLACE),
                        write_mask: vello::wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: vello::wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: vello::wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            },
        ));
        self.pipeline_format = Some(format);
    }
}

#[cfg(target_os = "android")]
#[derive(Clone)]
pub(crate) struct WgpuDetachedPresenter {
    surface: Arc<Mutex<super::runtime::RuntimeRenderSurface>>,
    blit_state: Arc<Mutex<DetachedBlitState>>,
}

#[cfg(target_os = "android")]
impl WgpuDetachedPresenter {
    pub(crate) fn new(
        surface: Arc<Mutex<super::runtime::RuntimeRenderSurface>>,
        blit_state: Arc<Mutex<DetachedBlitState>>,
    ) -> Self {
        Self {
            surface,
            blit_state,
        }
    }

    fn present_offscreen(self, frame: WgpuFrameContext) -> anyhow::Result<f64> {
        let present_t0 = std::time::Instant::now();
        let offscreen_texture = frame
            .offscreen_texture
            .ok_or_else(|| anyhow::anyhow!("Detached present missing offscreen texture"))?;

        let acquire_t0 = std::time::Instant::now();
        let (surface_texture, format) = {
            let surface = self
                .surface
                .lock()
                .map_err(|_| anyhow::anyhow!("Detached presenter surface lock poisoned"))?;
            let surface_texture = surface
                .surface
                .get_current_texture()
                .map_err(|e| anyhow::anyhow!("Detached presenter acquire failed: {:?}", e))?;
            (surface_texture, surface.format)
        };
        let late_acquire_ms = acquire_t0.elapsed().as_secs_f64() * 1000.0;

        let surface_view = surface_texture
            .texture
            .create_view(&vello::wgpu::TextureViewDescriptor::default());
        let offscreen_view =
            offscreen_texture.create_view(&vello::wgpu::TextureViewDescriptor::default());

        let mut blit_state = self
            .blit_state
            .lock()
            .map_err(|_| anyhow::anyhow!("Detached presenter blit state lock poisoned"))?;
        blit_state.ensure_pipeline(&frame.device, format);
        let layout = blit_state
            .layout
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Detached presenter layout missing"))?;
        let sampler = blit_state
            .sampler
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Detached presenter sampler missing"))?;
        let pipeline = blit_state
            .pipeline
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Detached presenter pipeline missing"))?;

        let bind_group = frame
            .device
            .create_bind_group(&vello::wgpu::BindGroupDescriptor {
                label: Some("Detached Presenter Blit Bind Group"),
                layout,
                entries: &[
                    vello::wgpu::BindGroupEntry {
                        binding: 0,
                        resource: vello::wgpu::BindingResource::TextureView(&offscreen_view),
                    },
                    vello::wgpu::BindGroupEntry {
                        binding: 1,
                        resource: vello::wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });

        let mut encoder =
            frame
                .device
                .create_command_encoder(&vello::wgpu::CommandEncoderDescriptor {
                    label: Some("Detached Presenter Blit Encoder"),
                });
        {
            let mut rp = encoder.begin_render_pass(&vello::wgpu::RenderPassDescriptor {
                label: Some("Detached Presenter Blit Pass"),
                color_attachments: &[Some(vello::wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: vello::wgpu::Operations {
                        load: vello::wgpu::LoadOp::Clear(vello::wgpu::Color::BLACK),
                        store: vello::wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, &bind_group, &[]);
            rp.draw(0..3, 0..1);
        }
        drop(blit_state);

        let blit_submit_t0 = std::time::Instant::now();
        let blit_submission = frame.queue.submit(Some(encoder.finish()));
        let blit_submit_ms = blit_submit_t0.elapsed().as_secs_f64() * 1000.0;

        let wait_for_blit = std::env::var("DYXEL_ANDROID_DETACHED_BLIT_READY_WAIT")
            .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
            .unwrap_or(false);
        let blit_wait_t0 = std::time::Instant::now();
        let blit_ready = if wait_for_blit {
            match frame.device.poll(vello::wgpu::PollType::Wait {
                submission_index: Some(blit_submission),
                timeout: Some(std::time::Duration::from_millis(50)),
            }) {
                Ok(_) => true,
                Err(vello::wgpu::PollError::Timeout) => false,
                Err(err) => {
                    log::warn!("[DIAG-RUNTIME] detached_surface_blit_wait error: {:?}", err);
                    false
                }
            }
        } else {
            false
        };
        let blit_wait_ms = blit_wait_t0.elapsed().as_secs_f64() * 1000.0;
        surface_texture.present();

        let present_ms = present_t0.elapsed().as_secs_f64() * 1000.0;
        if wait_for_blit && !blit_ready
            || late_acquire_ms >= 8.0
            || blit_wait_ms >= 4.0
            || present_ms >= 8.0
        {
            log::info!(
                "[DIAG-RUNTIME] detached_late_acquire_ms={:.2} detached_blit_submit_ms={:.2} detached_blit_wait_ms={:.2} wait={} ready={} detached_present_ms={:.2}",
                late_acquire_ms,
                blit_submit_ms,
                blit_wait_ms,
                wait_for_blit,
                blit_ready,
                present_ms
            );
        }
        Ok(present_ms)
    }
}

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
    /// The submission index of the last `queue.submit()` for this frame.
    /// Used by the presenter to wait only for this frame's GPU work.
    pub(crate) last_submission_index: Option<vello::wgpu::SubmissionIndex>,
    #[cfg(target_os = "android")]
    pub(crate) detached_presenter: Option<WgpuDetachedPresenter>,
}

impl BackendFrameContext for WgpuFrameContext {
    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn runtime_kind(&self) -> RuntimeKind {
        RuntimeKind::Wgpu
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn supports_detached_present(&self) -> bool {
        #[cfg(target_os = "android")]
        {
            return (self.render_to_offscreen && self.detached_presenter.is_some())
                || (!self.render_to_offscreen && self.surface_texture.is_some());
        }

        #[cfg(not(target_os = "android"))]
        {
            !self.render_to_offscreen && self.surface_texture.is_some()
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn present_detached(self: Box<Self>) -> anyhow::Result<f64> {
        let mut frame = *self;

        #[cfg(target_os = "android")]
        if frame.render_to_offscreen {
            let presenter = frame
                .detached_presenter
                .take()
                .ok_or_else(|| anyhow::anyhow!("WgpuFrameContext has no detached presenter"))?;
            return presenter.present_offscreen(frame);
        }

        if frame.render_to_offscreen {
            return Err(anyhow::anyhow!(
                "Detached present is only available for direct surface frames on this platform"
            ));
        }

        let present_t0 = std::time::Instant::now();
        let surface_texture = frame
            .surface_texture
            .take()
            .ok_or_else(|| anyhow::anyhow!("Detached direct present missing surface texture"))?;
        surface_texture.present();
        let present_ms = present_t0.elapsed().as_secs_f64() * 1000.0;
        if present_ms >= 8.0 {
            log::info!(
                "[DIAG-RUNTIME] detached_direct_present_ms={:.2}",
                present_ms
            );
        }
        Ok(present_ms)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn wait_until_gpu_ready(&self, timeout: std::time::Duration) -> anyhow::Result<bool> {
        // If no GPU work was submitted for this frame (e.g. w==0 || h==0),
        // there is nothing to wait for.
        let Some(submission_index) = self.last_submission_index.clone() else {
            return Ok(true);
        };
        match self.device.poll(vello::wgpu::PollType::Wait {
            submission_index: Some(submission_index),
            timeout: Some(timeout),
        }) {
            Ok(_) => Ok(true),
            Err(vello::wgpu::PollError::Timeout) => Ok(false),
            Err(err) => Err(anyhow::anyhow!("GPU ready wait failed: {:?}", err)),
        }
    }
}
