// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! WgpuRuntime — GraphicsRuntime implementation for Vello + wgpu
//!
//! Responsibilities:
//! - wgpu instance / device / queue lifecycle
//! - Surface creation / resize / suspend / resume from NativeSurfaceHandle
//! - Per-frame context acquisition (begin_frame) and present (end_frame)
//!
//! Does NOT execute scene drawing. Drawing is VelloBackend's job.

use dyxel_render_api::{
    BackendFrameContext, GraphicsRuntime, NativeSurfaceHandle, NativeSurfaceKind, RuntimeSurfaceId,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static RUNTIME_TIMING_DIAG_COUNTER: AtomicU64 = AtomicU64::new(0);
const RUNTIME_TIMING_SAMPLE_EVERY_N: u64 = 60;
const RUNTIME_TIMING_SAMPLE_THRESHOLD_MS: f64 = 1.0;
// Keep Android logcat pressure low. Surface present/acquire waits around
// 8–16ms are common under HWC backpressure; logging every such frame can itself
// perturb frame pacing. Truly pathological waits are still logged immediately,
// while moderate waits are sampled.
const RUNTIME_TIMING_ALWAYS_THRESHOLD_MS: f64 = 24.0;
const OFFSCREEN_FRAME_RING_LEN: usize = 3;

fn should_log_runtime_timing(ms: f64) -> bool {
    ms >= RUNTIME_TIMING_ALWAYS_THRESHOLD_MS
        || (ms >= RUNTIME_TIMING_SAMPLE_THRESHOLD_MS
            && RUNTIME_TIMING_DIAG_COUNTER.fetch_add(1, Ordering::Relaxed)
                % RUNTIME_TIMING_SAMPLE_EVERY_N
                == 0)
}

#[cfg(target_os = "android")]
fn android_full_frame_offscreen_enabled() -> bool {
    std::env::var("DYXEL_ANDROID_FULL_FRAME_OFFSCREEN")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "android"))]
fn android_full_frame_offscreen_enabled() -> bool {
    false
}

#[cfg(target_os = "android")]
fn android_surface_ready_wait_enabled() -> bool {
    std::env::var("DYXEL_ANDROID_SURFACE_READY_WAIT")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
        .unwrap_or(false)
}

#[cfg(target_os = "android")]
fn android_surface_ready_wait_timeout() -> std::time::Duration {
    let timeout_ms = std::env::var("DYXEL_ANDROID_SURFACE_READY_WAIT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(6);
    std::time::Duration::from_millis(timeout_ms)
}

#[cfg(target_os = "android")]
fn android_present_mode() -> wgpu::PresentMode {
    match std::env::var("DYXEL_ANDROID_PRESENT_MODE")
        .unwrap_or_else(|_| "mailbox".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "fifo" => wgpu::PresentMode::Fifo,
        "immediate" => wgpu::PresentMode::Immediate,
        _ => wgpu::PresentMode::Mailbox,
    }
}

#[cfg(target_os = "android")]
fn android_surface_max_frame_latency() -> u32 {
    std::env::var("DYXEL_ANDROID_MAX_FRAME_LATENCY")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(2)
        .clamp(1, 3)
}

#[cfg(target_os = "android")]
fn android_force_opaque_surface() -> bool {
    std::env::var("DYXEL_ANDROID_FORCE_OPAQUE_SURFACE")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
        .unwrap_or(false)
}

#[cfg(target_os = "android")]
fn android_offscreen_copy_present_enabled() -> bool {
    std::env::var("DYXEL_ANDROID_OFFSCREEN_COPY_PRESENT")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
        .unwrap_or(false)
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// Wgpu graphics runtime — manages the wgpu instance, device, queue, and surfaces.
pub struct WgpuRuntime {
    instance: wgpu::Instance,
    render_context: RuntimeRenderContext,
    surfaces: HashMap<RuntimeSurfaceId, Arc<Mutex<RuntimeRenderSurface>>>,
    #[cfg(target_os = "android")]
    native_presenters:
        HashMap<RuntimeSurfaceId, super::android_native_presenter::AndroidNativePresenterProbe>,
    offscreen_targets: HashMap<RuntimeSurfaceId, RuntimeOffscreenTarget>,
    next_surface_id: u32,
    late_blit_layout: Option<wgpu::BindGroupLayout>,
    late_blit_shader: Option<wgpu::ShaderModule>,
    late_blit_pipeline: Option<wgpu::RenderPipeline>,
    late_blit_pipeline_format: Option<wgpu::TextureFormat>,
    late_blit_sampler: Option<wgpu::Sampler>,
    #[cfg(target_os = "android")]
    detached_blit_state: Arc<Mutex<super::frame_context::DetachedBlitState>>,
}

struct RuntimeOffscreenTarget {
    slots: Vec<RuntimeOffscreenSlot>,
    next_slot: usize,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

struct RuntimeOffscreenSlot {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub(crate) struct RuntimeRenderContext {
    pub(crate) instance: wgpu::Instance,
    pub(crate) devices: Vec<RuntimeDeviceHandle>,
}

pub(crate) struct RuntimeDeviceHandle {
    adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

pub(crate) struct RuntimeRenderSurface {
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) config: wgpu::SurfaceConfiguration,
    pub(crate) dev_id: usize,
    pub(crate) format: wgpu::TextureFormat,
}

impl RuntimeRenderContext {
    fn new() -> Self {
        let backends = wgpu::Backends::from_env().unwrap_or_default();
        let flags = wgpu::InstanceFlags::from_build_config().with_env();
        let memory_budget_thresholds = wgpu::MemoryBudgetThresholds::default();
        let backend_options = wgpu::BackendOptions::from_env_or_default();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            flags,
            memory_budget_thresholds,
            backend_options,
        });
        Self {
            instance,
            devices: Vec::new(),
        }
    }

    async fn create_render_surface(
        &mut self,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        present_mode: wgpu::PresentMode,
    ) -> anyhow::Result<RuntimeRenderSurface> {
        let dev_id = self
            .device(Some(&surface))
            .await
            .ok_or_else(|| anyhow::anyhow!("No compatible wgpu device found"))?;

        let device_handle = &self.devices[dev_id];
        let capabilities = surface.get_capabilities(&device_handle.adapter);
        let format = capabilities
            .formats
            .into_iter()
            .find(|it| {
                matches!(
                    it,
                    wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
                )
            })
            .ok_or_else(|| anyhow::anyhow!("Unsupported surface format"))?;

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
        };
        let surface = RuntimeRenderSurface {
            surface,
            config,
            dev_id,
            format,
        };
        self.configure_surface(&surface);
        Ok(surface)
    }

    fn resize_surface(&self, surface: &mut RuntimeRenderSurface, width: u32, height: u32) {
        surface.config.width = width;
        surface.config.height = height;
        self.configure_surface(surface);
    }

    fn configure_surface(&self, surface: &RuntimeRenderSurface) {
        let device = &self.devices[surface.dev_id].device;
        surface.surface.configure(device, &surface.config);
    }

    async fn device(&mut self, compatible_surface: Option<&wgpu::Surface<'_>>) -> Option<usize> {
        let compatible = match compatible_surface {
            Some(surface) => self
                .devices
                .iter()
                .enumerate()
                .find(|(_, device)| device.adapter.is_surface_supported(surface))
                .map(|(index, _)| index),
            None => (!self.devices.is_empty()).then_some(0),
        };
        if compatible.is_none() {
            return self.new_device(compatible_surface).await;
        }
        compatible
    }

    async fn new_device(
        &mut self,
        compatible_surface: Option<&wgpu::Surface<'_>>,
    ) -> Option<usize> {
        let adapter =
            wgpu::util::initialize_adapter_from_env_or_default(&self.instance, compatible_surface)
                .await
                .ok()?;
        let features = adapter.features();
        let limits = wgpu::Limits::default();
        let maybe_features = wgpu::Features::CLEAR_TEXTURE | wgpu::Features::PIPELINE_CACHE;

        #[cfg(target_os = "android")]
        if super::android_native_presenter::android_native_presenter_custom_device_enabled() {
            let requested_features =
                super::android_native_presenter::default_native_presenter_wgpu_features(&adapter);
            match super::android_native_presenter::create_wgpu_custom_vulkan_device_with_android_interop(
                &adapter,
                requested_features,
                "Dyxel Android native presenter main custom Vulkan device",
            ) {
                Ok((device, queue)) => {
                    log::warn!(
                        "[DIAG-NATIVE-PRESENTER] using experimental custom Vulkan wgpu Device for main renderer"
                    );
                    self.devices.push(RuntimeDeviceHandle {
                        adapter,
                        device,
                        queue,
                    });
                    return Some(self.devices.len() - 1);
                }
                Err(err) => {
                    log::warn!(
                        "[DIAG-NATIVE-PRESENTER] custom Vulkan main Device failed, falling back to default wgpu device: {:?}",
                        err
                    );
                }
            }
        }

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: features & maybe_features,
                required_limits: limits,
                ..Default::default()
            })
            .await
            .ok()?;
        self.devices.push(RuntimeDeviceHandle {
            adapter,
            device,
            queue,
        });
        Some(self.devices.len() - 1)
    }
}

impl WgpuRuntime {
    pub fn new() -> Self {
        let render_context = RuntimeRenderContext::new();
        let instance = render_context.instance.clone();
        Self {
            instance,
            render_context,
            surfaces: HashMap::new(),
            #[cfg(target_os = "android")]
            native_presenters: HashMap::new(),
            offscreen_targets: HashMap::new(),
            next_surface_id: 1,
            late_blit_layout: None,
            late_blit_shader: None,
            late_blit_pipeline: None,
            late_blit_pipeline_format: None,
            late_blit_sampler: None,
            #[cfg(target_os = "android")]
            detached_blit_state: Arc::new(Mutex::new(
                super::frame_context::DetachedBlitState::new(),
            )),
        }
    }

    /// Get a reference to the wgpu device for the first (and usually only) device.
    pub fn device(&self) -> Option<&wgpu::Device> {
        self.render_context.devices.first().map(|d| &d.device)
    }

    /// Get a reference to the wgpu queue for the first device.
    pub fn queue(&self) -> Option<&wgpu::Queue> {
        self.render_context.devices.first().map(|d| &d.queue)
    }

    fn ensure_late_blit_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.late_blit_pipeline.is_some() && self.late_blit_pipeline_format == Some(format) {
            return;
        }

        let shader = self.late_blit_shader.get_or_insert_with(|| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Runtime Late Surface Blit Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
            })
        });

        let layout = self.late_blit_layout.get_or_insert_with(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Runtime Late Surface Blit BGL"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            })
        });

        self.late_blit_sampler.get_or_insert_with(|| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Runtime Late Surface Blit Sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Runtime Late Surface Blit Pipeline Layout"),
            bind_group_layouts: &[layout],
            push_constant_ranges: &[],
        });
        self.late_blit_pipeline = Some(device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("Runtime Late Surface Blit Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            },
        ));
        self.late_blit_pipeline_format = Some(format);
    }

    fn next_offscreen_target(
        &mut self,
        surface_id: RuntimeSurfaceId,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let needs_recreate = self.offscreen_targets.get(&surface_id).map_or(true, |t| {
            t.width != width
                || t.height != height
                || t.format != format
                || t.slots.len() != OFFSCREEN_FRAME_RING_LEN
        });
        if needs_recreate {
            let mut slots = Vec::with_capacity(OFFSCREEN_FRAME_RING_LEN);
            for _ in 0..OFFSCREEN_FRAME_RING_LEN {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Runtime Offscreen Frame Target"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("Runtime Offscreen Frame Target View"),
                    ..Default::default()
                });
                slots.push(RuntimeOffscreenSlot { texture, view });
            }
            self.offscreen_targets.insert(
                surface_id,
                RuntimeOffscreenTarget {
                    slots,
                    next_slot: 0,
                    width,
                    height,
                    format,
                },
            );
            log::info!(
                "[DIAG-RUNTIME] recreated offscreen frame target {}x{} {:?}",
                width,
                height,
                format
            );
        }
        let target = self
            .offscreen_targets
            .get_mut(&surface_id)
            .expect("offscreen target must exist after creation");
        let slot_index = target.next_slot % target.slots.len();
        target.next_slot = (target.next_slot + 1) % target.slots.len();
        let slot = &target.slots[slot_index];
        (slot.texture.clone(), slot.view.clone())
    }

    #[allow(dead_code)]
    fn offscreen_target_for_tests(&self, surface_id: RuntimeSurfaceId) -> &RuntimeOffscreenTarget {
        self.offscreen_targets
            .get(&surface_id)
            .expect("offscreen target must exist after creation")
    }
}

impl GraphicsRuntime for WgpuRuntime {
    fn initialize(&mut self) -> anyhow::Result<()> {
        // Ensure at least one device is available.
        if self.render_context.devices.is_empty() {
            let dev_id = pollster::block_on(async { self.render_context.device(None).await })
                .ok_or_else(|| anyhow::anyhow!("No compatible wgpu device found"))?;
            log::info!("WgpuRuntime: initialized device id {}", dev_id);
        }
        Ok(())
    }

    fn create_surface(
        &mut self,
        handle: NativeSurfaceHandle,
        width: u32,
        height: u32,
    ) -> anyhow::Result<RuntimeSurfaceId> {
        #[cfg(target_os = "android")]
        let mut android_native_window_ptr: Option<u64> = None;

        let surface = match handle {
            NativeSurfaceHandle::RawWindow { window, display } => {
                let target = wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_window_handle: window,
                    raw_display_handle: display,
                };
                unsafe { self.instance.create_surface_unsafe(target) }.map_err(|e| {
                    anyhow::anyhow!("Failed to create wgpu surface from raw handle: {:?}", e)
                })?
            }
            NativeSurfaceHandle::WebCanvas { canvas_id } => {
                #[cfg(target_arch = "wasm32")]
                {
                    let window = web_sys::window()
                        .ok_or_else(|| anyhow::anyhow!("No web window available"))?;
                    let document = window
                        .document()
                        .ok_or_else(|| anyhow::anyhow!("No document available"))?;
                    let canvas = document.get_element_by_id(&canvas_id).ok_or_else(|| {
                        anyhow::anyhow!("Canvas element '{}' not found", canvas_id)
                    })?;
                    let canvas: web_sys::HtmlCanvasElement = canvas
                        .dyn_into()
                        .map_err(|_| anyhow::anyhow!("Element '{}' is not a canvas", canvas_id))?;
                    let target = wgpu::SurfaceTarget::Canvas(canvas);
                    self.instance.create_surface(target).map_err(|e| {
                        anyhow::anyhow!("Failed to create wgpu surface from canvas: {:?}", e)
                    })?
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    return Err(anyhow::anyhow!(
                        "WebCanvas surface creation is only supported on wasm32 (got canvas_id: {})",
                        canvas_id
                    ));
                }
            }
            NativeSurfaceHandle::NativeSurface { kind, ptr } => match kind {
                NativeSurfaceKind::Android => {
                    #[cfg(target_os = "android")]
                    {
                        android_native_window_ptr = Some(ptr);
                    }
                    let handle = raw_window_handle::AndroidNdkWindowHandle::new(
                        std::ptr::NonNull::new(ptr as *mut std::ffi::c_void)
                            .ok_or_else(|| anyhow::anyhow!("Invalid ANativeWindow pointer"))?,
                    );
                    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
                        raw_window_handle: raw_window_handle::RawWindowHandle::AndroidNdk(handle),
                        raw_display_handle: raw_window_handle::RawDisplayHandle::Android(
                            raw_window_handle::AndroidDisplayHandle::new(),
                        ),
                    };
                    unsafe { self.instance.create_surface_unsafe(target) }
                        .map_err(|e| anyhow::anyhow!("Failed to create Android surface: {:?}", e))?
                }
                _ => {
                    return Err(anyhow::anyhow!(
                        "NativeSurface kind {:?} not yet supported in WgpuRuntime",
                        kind
                    ));
                }
            },
        };

        // Select present mode based on platform capabilities.
        // Android supports Mailbox (low-latency VSync replacement), but Fifo
        // can be useful to validate whether HWC fence waits come from mailbox
        // replacement timing rather than GPU work itself.
        #[cfg(target_os = "android")]
        let present_mode = android_present_mode();
        #[cfg(not(target_os = "android"))]
        let present_mode = wgpu::PresentMode::Fifo;

        #[cfg_attr(not(target_os = "android"), allow(unused_mut))]
        let mut v_surface = pollster::block_on(async {
            self.render_context
                .create_render_surface(surface, width, height, present_mode)
                .await
        })
        .map_err(|e| anyhow::anyhow!("Failed to create render surface: {:?}", e))?;

        let id = RuntimeSurfaceId(self.next_surface_id);
        self.next_surface_id += 1;
        let dev_id = v_surface.dev_id;

        #[cfg(target_os = "android")]
        {
            let device_handle = &self.render_context.devices[dev_id];
            let caps = v_surface.surface.get_capabilities(&device_handle.adapter);
            let requested_alpha = if android_force_opaque_surface()
                && caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque)
            {
                wgpu::CompositeAlphaMode::Opaque
            } else {
                v_surface.config.alpha_mode
            };
            let requested_latency = android_surface_max_frame_latency();
            if android_offscreen_copy_present_enabled() {
                v_surface.config.usage |= wgpu::TextureUsages::COPY_DST;
            }
            if v_surface.config.desired_maximum_frame_latency != requested_latency
                || v_surface.config.alpha_mode != requested_alpha
                || android_offscreen_copy_present_enabled()
            {
                v_surface.config.desired_maximum_frame_latency = requested_latency;
                v_surface.config.alpha_mode = requested_alpha;
                v_surface
                    .surface
                    .configure(&device_handle.device, &v_surface.config);
                log::info!(
                    "[DIAG-RUNTIME] Android surface config override latency={} alpha={:?} usage={:?}",
                    requested_latency,
                    requested_alpha,
                    v_surface.config.usage
                );
            }
        }

        self.surfaces.insert(id, Arc::new(Mutex::new(v_surface)));

        #[cfg(target_os = "android")]
        {
            let native_presenter_requested =
                super::android_native_presenter::android_native_presenter_enabled();
            let native_presenter_diag_requested =
                super::android_native_presenter::android_native_presenter_diag_enabled();
            let custom_device_probe_requested =
                super::android_native_presenter::android_native_presenter_custom_device_probe_enabled();
            let ahb_import_probe_requested =
                super::android_native_presenter::android_native_presenter_ahb_import_probe_enabled(
                );
            let gpu_clear_probe_requested =
                super::android_native_presenter::android_native_presenter_gpu_clear_probe_enabled();
            let gpu_present_probe_requested =
                super::android_native_presenter::android_native_presenter_gpu_present_probe_enabled(
                );
            let wgpu_ahb_texture_probe_requested =
                super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_texture_probe_enabled();
            let wgpu_ahb_frame_requested =
                super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_frame_enabled();
            if native_presenter_requested
                || native_presenter_diag_requested
                || custom_device_probe_requested
                || ahb_import_probe_requested
                || gpu_clear_probe_requested
                || gpu_present_probe_requested
                || wgpu_ahb_texture_probe_requested
                || wgpu_ahb_frame_requested
            {
                super::android_native_presenter::log_android_cpu_ahb_presenter_support();
                let device_handle = &self.render_context.devices[dev_id];
                super::android_native_presenter::log_wgpu_vulkan_external_ahb_support(
                    &device_handle.adapter,
                    &device_handle.device,
                );
                if custom_device_probe_requested {
                    super::android_native_presenter::probe_wgpu_custom_vulkan_device_extensions(
                        &device_handle.adapter,
                    );
                }
                if ahb_import_probe_requested {
                    super::android_native_presenter::probe_wgpu_vulkan_ahb_import(
                        &device_handle.adapter,
                        &device_handle.device,
                    );
                }
                if gpu_clear_probe_requested {
                    super::android_native_presenter::probe_wgpu_vulkan_ahb_gpu_clear(
                        &device_handle.adapter,
                        &device_handle.device,
                    );
                }
            }
        }

        #[cfg(target_os = "android")]
        if super::android_native_presenter::android_native_presenter_enabled()
            || super::android_native_presenter::android_native_presenter_gpu_present_probe_enabled()
            || super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_texture_probe_enabled()
            || super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_frame_enabled()
        {
            if let Some(ptr) = android_native_window_ptr {
                match super::android_native_presenter::AndroidNativePresenterProbe::new_from_anative_window_ptr(
                    ptr,
                    if super::android_native_presenter::android_native_presenter_enabled()
                        || super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_frame_enabled()
                    { width } else { width.min(96) },
                    if super::android_native_presenter::android_native_presenter_enabled()
                        || super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_frame_enabled()
                    { height } else { height.min(96) },
                ) {
                    Ok(mut presenter) => {
                        if super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_texture_probe_enabled() {
                            let device_handle = &self.render_context.devices[dev_id];
                            if let Err(err) = presenter.submit_wgpu_ahb_texture_probe_once(
                                &device_handle.adapter,
                                &device_handle.device,
                                &device_handle.queue,
                            ) {
                                log::warn!(
                                    "[DIAG-NATIVE-PRESENTER] wgpu AHB texture SurfaceControl probe failed: {:?}",
                                    err
                                );
                            }
                        } else if super::android_native_presenter::android_native_presenter_gpu_present_probe_enabled() {
                            let device_handle = &self.render_context.devices[dev_id];
                            if let Err(err) = presenter.submit_gpu_clear_probe_once(
                                &device_handle.adapter,
                                &device_handle.device,
                            ) {
                                log::warn!(
                                    "[DIAG-NATIVE-PRESENTER] GPU SurfaceControl present probe failed: {:?}",
                                    err
                                );
                            }
                        }
                        self.native_presenters.insert(id, presenter);
                    }
                    Err(err) => {
                        log::warn!(
                            "[DIAG-NATIVE-PRESENTER] disabled native presenter probe after init failure: {:?}",
                            err
                        );
                    }
                }
            } else {
                log::warn!(
                    "[DIAG-NATIVE-PRESENTER] native presenter requested but no ANativeWindow pointer was available"
                );
            }
        }

        log::info!(
            "WgpuRuntime: created surface {:?} ({}x{}) dev_id={} present_mode={:?}",
            id,
            width,
            height,
            dev_id,
            present_mode
        );
        Ok(id)
    }

    fn resize_surface(
        &mut self,
        surface_id: RuntimeSurfaceId,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let surface = self
            .surfaces
            .get(&surface_id)
            .ok_or_else(|| anyhow::anyhow!("Surface {:?} not found", surface_id))?;
        let mut surface = surface
            .lock()
            .map_err(|_| anyhow::anyhow!("Surface {:?} lock poisoned", surface_id))?;
        self.render_context
            .resize_surface(&mut surface, width, height);
        #[cfg(target_os = "android")]
        let dev_id = surface.dev_id;
        drop(surface);
        self.offscreen_targets.remove(&surface_id);
        #[cfg(target_os = "android")]
        if let Some(presenter) = self.native_presenters.get_mut(&surface_id) {
            let (presenter_width, presenter_height) =
                if super::android_native_presenter::android_native_presenter_enabled()
                    || super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_frame_enabled()
                {
                    (width, height)
                } else {
                    (width.min(96), height.min(96))
                };
            if let Err(err) = presenter.resize(presenter_width, presenter_height) {
                log::warn!(
                    "[DIAG-NATIVE-PRESENTER] resize ignored after failure: {:?}",
                    err
                );
                self.native_presenters.remove(&surface_id);
            } else if super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_texture_probe_enabled() {
                let device_handle = &self.render_context.devices[dev_id];
                if let Err(err) = presenter.submit_wgpu_ahb_texture_probe_once(
                    &device_handle.adapter,
                    &device_handle.device,
                    &device_handle.queue,
                ) {
                    log::warn!(
                        "[DIAG-NATIVE-PRESENTER] wgpu AHB texture SurfaceControl probe after resize failed: {:?}",
                        err
                    );
                }
            } else if super::android_native_presenter::android_native_presenter_gpu_present_probe_enabled() {
                let device_handle = &self.render_context.devices[dev_id];
                if let Err(err) = presenter.submit_gpu_clear_probe_once(
                    &device_handle.adapter,
                    &device_handle.device,
                )
                {
                    log::warn!(
                        "[DIAG-NATIVE-PRESENTER] GPU SurfaceControl present probe after resize failed: {:?}",
                        err
                    );
                }
            }
        }
        // resize_surface already calls configure_surface internally
        Ok(())
    }

    fn suspend(&mut self) -> anyhow::Result<()> {
        for (_, surface) in &mut self.surfaces {
            let surface = surface
                .lock()
                .map_err(|_| anyhow::anyhow!("Surface lock poisoned during suspend"))?;
            surface.surface.configure(
                &self.render_context.devices[surface.dev_id].device,
                &wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: surface.format,
                    width: surface.config.width,
                    height: surface.config.height,
                    present_mode: surface.config.present_mode,
                    desired_maximum_frame_latency: surface.config.desired_maximum_frame_latency,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: vec![],
                },
            );
        }
        #[cfg(target_os = "android")]
        self.native_presenters.clear();
        Ok(())
    }

    fn resume(&mut self) -> anyhow::Result<()> {
        for (_, surface) in &mut self.surfaces {
            let surface = surface
                .lock()
                .map_err(|_| anyhow::anyhow!("Surface lock poisoned during resume"))?;
            let device = &self.render_context.devices[surface.dev_id].device;
            surface.surface.configure(device, &surface.config);
        }
        Ok(())
    }

    fn sync_gpu(&mut self) -> anyhow::Result<()> {
        if let Some(device) = self.device() {
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        }
        Ok(())
    }

    fn begin_frame(
        &mut self,
        surface_id: RuntimeSurfaceId,
    ) -> anyhow::Result<Box<dyn BackendFrameContext>> {
        let (dev_id, format, width, height) = {
            let surface = self
                .surfaces
                .get(&surface_id)
                .ok_or_else(|| anyhow::anyhow!("Surface {:?} not found", surface_id))?;
            let surface = surface
                .lock()
                .map_err(|_| anyhow::anyhow!("Surface {:?} lock poisoned", surface_id))?;
            (
                surface.dev_id,
                surface.format,
                surface.config.width,
                surface.config.height,
            )
        };
        let device = self.render_context.devices[dev_id].device.clone();
        let queue = self.render_context.devices[dev_id].queue.clone();

        #[cfg(any(target_os = "macos", target_os = "android"))]
        {
            // macOS/Fifo benefits from full-frame offscreen-first because
            // drawable acquire can block. On Android this is an experiment only:
            // real-device logs showed the extra full-frame blit can push mobile
            // GPU/HWC over budget and drop the app to ~40fps, so keep it opt-in.
            #[cfg(target_os = "android")]
            let native_ahb_frame =
                super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_frame_enabled();
            #[cfg(not(target_os = "android"))]
            let native_ahb_frame = false;
            let use_offscreen_first = cfg!(target_os = "macos")
                || android_full_frame_offscreen_enabled()
                || native_ahb_frame;
            if use_offscreen_first {
                let (offscreen_texture, offscreen_view) =
                    self.next_offscreen_target(surface_id, &device, width, height, format);
                #[cfg(target_os = "android")]
                let detached_presenter = {
                    let surface = self
                        .surfaces
                        .get(&surface_id)
                        .ok_or_else(|| anyhow::anyhow!("Surface {:?} not found", surface_id))?
                        .clone();
                    Some(super::frame_context::WgpuDetachedPresenter::new(
                        surface,
                        self.detached_blit_state.clone(),
                    ))
                };
                #[cfg(not(target_os = "android"))]
                let detached_presenter = None;

                return Ok(Box::new(super::frame_context::WgpuFrameContext {
                    surface_id,
                    surface_texture: None,
                    offscreen_texture: Some(offscreen_texture),
                    view: offscreen_view,
                    render_to_offscreen: true,
                    device,
                    queue,
                    format,
                    width,
                    height,
                    acquire_ms: 0.0,
                    present_ms: 0.0,
                    last_submission_index: None,
                    detached_presenter,
                }));
            }
        }

        {
            // Acquire surface texture for this frame.
            let acquire_t0 = std::time::Instant::now();
            let surface = self
                .surfaces
                .get(&surface_id)
                .ok_or_else(|| anyhow::anyhow!("Surface {:?} not found", surface_id))?;
            let surface_texture = surface
                .lock()
                .map_err(|_| anyhow::anyhow!("Surface {:?} lock poisoned", surface_id))?
                .surface
                .get_current_texture()
                .map_err(|e| anyhow::anyhow!("Failed to acquire surface texture: {:?}", e))?;
            let acquire_ms = acquire_t0.elapsed().as_secs_f64() * 1000.0;
            if should_log_runtime_timing(acquire_ms) {
                log::info!("[DIAG-RUNTIME] acquire_ms={:.2}", acquire_ms);
            }

            let view = surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            Ok(Box::new(super::frame_context::WgpuFrameContext {
                surface_id,
                surface_texture: Some(surface_texture),
                offscreen_texture: None,
                view,
                render_to_offscreen: false,
                device,
                queue,
                format,
                width,
                height,
                acquire_ms,
                present_ms: 0.0,
                last_submission_index: None,
                detached_presenter: None,
            }))
        }
    }

    fn end_frame(&mut self, mut frame: Box<dyn BackendFrameContext>) -> anyhow::Result<()> {
        let frame = frame
            .as_any()
            .downcast_mut::<super::frame_context::WgpuFrameContext>()
            .ok_or_else(|| anyhow::anyhow!("Invalid frame context type"))?;

        let present_t0 = std::time::Instant::now();

        if frame.render_to_offscreen {
            #[cfg(target_os = "android")]
            if super::android_native_wgpu_ahb::android_native_presenter_wgpu_ahb_frame_enabled() {
                if let Some(presenter) = self.native_presenters.get_mut(&frame.surface_id) {
                    let offscreen_texture = frame.offscreen_texture.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("Missing offscreen texture for native AHB frame")
                    })?;
                    match super::android_native_wgpu_ahb::present_offscreen_texture_frame(
                        presenter,
                        &frame.device,
                        &frame.queue,
                        offscreen_texture,
                        frame.width,
                        frame.height,
                    ) {
                        Ok(present_ms) => {
                            frame.present_ms = present_ms;
                            return Ok(());
                        }
                        Err(err) => {
                            log::warn!(
                                "[DIAG-NATIVE-PRESENTER] native AHB frame present failed, falling back to wgpu Surface: {:?}",
                                err
                            );
                        }
                    }
                } else {
                    log::warn!(
                        "[DIAG-NATIVE-PRESENTER] native AHB frame requested but presenter is missing; falling back to wgpu Surface"
                    );
                }
            }

            let surface = self
                .surfaces
                .get(&frame.surface_id)
                .ok_or_else(|| anyhow::anyhow!("Surface {:?} not found", frame.surface_id))?;

            let acquire_t0 = std::time::Instant::now();
            let (surface_texture, surface_format) = {
                let surface = surface
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Surface {:?} lock poisoned", frame.surface_id))?;
                let surface_texture = surface.surface.get_current_texture().map_err(|e| {
                    anyhow::anyhow!("Failed to late-acquire surface texture: {:?}", e)
                })?;
                (surface_texture, surface.format)
            };
            let late_acquire_ms = acquire_t0.elapsed().as_secs_f64() * 1000.0;

            #[cfg(target_os = "android")]
            if android_offscreen_copy_present_enabled() {
                let copy_t0 = std::time::Instant::now();
                let offscreen_texture = frame
                    .offscreen_texture
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Missing offscreen texture in frame context"))?;
                let mut encoder =
                    frame
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Runtime Late Surface Copy Encoder"),
                        });
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: offscreen_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &surface_texture.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: frame.width,
                        height: frame.height,
                        depth_or_array_layers: 1,
                    },
                );
                frame.queue.submit(Some(encoder.finish()));
                surface_texture.present();
                let end_ms = present_t0.elapsed().as_secs_f64() * 1000.0;
                let copy_ms = copy_t0.elapsed().as_secs_f64() * 1000.0;
                if should_log_runtime_timing(late_acquire_ms)
                    || should_log_runtime_timing(copy_ms)
                    || should_log_runtime_timing(end_ms)
                {
                    log::info!(
                        "[DIAG-RUNTIME] late_acquire_ms={:.2} late_copy_present_ms={:.2} copy_submit_present_ms={:.2}",
                        late_acquire_ms,
                        end_ms,
                        copy_ms
                    );
                }
                frame.present_ms = present_t0.elapsed().as_secs_f64() * 1000.0;
                if should_log_runtime_timing(frame.present_ms) {
                    log::info!("[DIAG-RUNTIME] present_ms={:.2}", frame.present_ms);
                }
                return Ok(());
            }

            let surface_view = surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let offscreen_view = frame
                .offscreen_texture
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing offscreen texture in frame context"))?
                .create_view(&wgpu::TextureViewDescriptor::default());

            self.ensure_late_blit_pipeline(&frame.device, surface_format);
            let bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Runtime Late Surface Blit Bind Group"),
                layout: self
                    .late_blit_layout
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Late blit layout not initialized"))?,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&offscreen_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(
                            self.late_blit_sampler.as_ref().ok_or_else(|| {
                                anyhow::anyhow!("Late blit sampler not initialized")
                            })?,
                        ),
                    },
                ],
            });

            let mut encoder =
                frame
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Runtime Late Surface Blit Encoder"),
                    });
            {
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Runtime Late Surface Blit Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &surface_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                rp.set_pipeline(
                    self.late_blit_pipeline
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Late blit pipeline not initialized"))?,
                );
                rp.set_bind_group(0, &bind_group, &[]);
                rp.draw(0..3, 0..1);
            }
            frame.queue.submit(Some(encoder.finish()));
            surface_texture.present();

            let end_ms = present_t0.elapsed().as_secs_f64() * 1000.0;
            if should_log_runtime_timing(late_acquire_ms) || should_log_runtime_timing(end_ms) {
                log::info!(
                    "[DIAG-RUNTIME] late_acquire_ms={:.2} late_blit_present_ms={:.2}",
                    late_acquire_ms,
                    end_ms,
                );
            }
        } else if let Some(surface_texture) = frame.surface_texture.take() {
            #[cfg(target_os = "android")]
            if android_surface_ready_wait_enabled() {
                if let Some(submission_index) = frame.last_submission_index.clone() {
                    let wait_t0 = std::time::Instant::now();
                    let ready = match frame.device.poll(wgpu::PollType::Wait {
                        submission_index: Some(submission_index),
                        timeout: Some(android_surface_ready_wait_timeout()),
                    }) {
                        Ok(_) => true,
                        Err(wgpu::PollError::Timeout) => false,
                        Err(err) => {
                            log::warn!("[DIAG-RUNTIME] surface_ready_wait error: {:?}", err);
                            false
                        }
                    };
                    let wait_ms = wait_t0.elapsed().as_secs_f64() * 1000.0;
                    if should_log_runtime_timing(wait_ms) {
                        log::info!(
                            "[DIAG-RUNTIME] surface_ready_wait_ms={:.2} ready={}",
                            wait_ms,
                            ready
                        );
                    }
                }
            }
            surface_texture.present();
        }
        frame.present_ms = present_t0.elapsed().as_secs_f64() * 1000.0;
        if should_log_runtime_timing(frame.present_ms) {
            log::info!("[DIAG-RUNTIME] present_ms={:.2}", frame.present_ms);
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
