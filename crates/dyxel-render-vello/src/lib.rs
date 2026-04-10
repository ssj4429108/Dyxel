// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_perf::{PerfConfig, PerformanceDiagnostics, PerformanceMonitor, SharedPerfMonitor};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use dyxel_render_api::{
    BackendConfig, DeviceHandle, LifecycleEvent, QueueHandle, RenderBackend, RenderBackendExt,
    RenderContext, RenderResult, SharedMutex, SharedPtr, SurfaceHandle, SurfaceState,
    SurfaceTargetHandle, VelloBackendExt,
};
use dyxel_shared::{SharedState, ViewType};
use kurbo::{Affine, Rect as KRect, Vec2};
use std::any::Any;
use std::sync::atomic::AtomicBool;
use taffy::style::AvailableSpace;
use vello::wgpu;
use vello::{
    peniko::{Color, Fill},
    Renderer, RendererOptions, Scene,
};

// Re-export TextInputRenderState from dyxel-render-api
pub use dyxel_render_api::TextInputRenderState;

// Global text input states for cursor rendering
// This is accessed from dyxel-core via exported functions
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static TEXT_INPUT_STATES: RefCell<HashMap<u32, TextInputRenderState>> = RefCell::new(HashMap::new());
}

/// Update global text input state
pub fn update_text_input_state_global(node_id: u32, state: TextInputRenderState) {
    TEXT_INPUT_STATES.with(|s| {
        s.borrow_mut().insert(node_id, state);
    });
}

/// Remove global text input state
pub fn remove_text_input_state_global(node_id: u32) {
    TEXT_INPUT_STATES.with(|s| {
        s.borrow_mut().remove(&node_id);
    });
}

/// Get global text input state
pub fn get_text_input_state_global(node_id: u32) -> Option<TextInputRenderState> {
    TEXT_INPUT_STATES.with(|s| s.borrow().get(&node_id).cloned())
}

/// Clear all global text input states
pub fn clear_text_input_states_global() {
    TEXT_INPUT_STATES.with(|s| s.borrow_mut().clear());
}

/// Get all text input states as a HashMap
pub fn get_all_text_input_states() -> HashMap<u32, TextInputRenderState> {
    TEXT_INPUT_STATES.with(|s| s.borrow().clone())
}

use dyxel_editor::Editor;
// Two-stage init is implemented inline with cache header markers

#[cfg(target_os = "android")]
pub mod android;
pub mod keyboard;
#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_arch = "wasm32")]
pub mod web;

pub mod filter_pipeline;
pub mod minimal_shaders;
pub mod scene_adapter;
pub mod shader_cache;
pub mod staged_init;
pub mod staged_loader;
pub mod two_stage_init;

/// Vello render backend implementation
///
/// This is the concrete implementation of RenderBackend using Vello + wgpu
// Type aliases for shared data used in async context
type AsyncShared<T> = std::sync::Arc<std::sync::Mutex<T>>;

/// Entry for a blurred texture to be composited
#[derive(Debug)]
#[allow(dead_code)]
struct BlurredTextureEntry {
    /// The blurred texture
    texture: wgpu::Texture,
    /// Width of the texture
    width: u32,
    /// Height of the texture
    height: u32,
    /// Position to draw at (with padding offset already applied)
    transform: Affine,
    /// Opacity of the blurred content
    opacity: f32,
}

/// Vello render backend implementation
///
/// This is the concrete implementation of RenderBackend using Vello + wgpu
pub struct VelloBackend {
    pub renderer: AsyncShared<Option<Renderer>>,
    pub blit_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    pub sampler: SharedMutex<Option<wgpu::Sampler>>,
    pub blit_shader: SharedMutex<Option<wgpu::ShaderModule>>,
    pub blit_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    pub pipeline_cache: AsyncShared<Option<wgpu::PipelineCache>>,
    pub cache_path: AsyncShared<Option<String>>,
    pub cache_saved: AtomicBool,
    // Current cache stage: None = no cache, Some(1) = Stage 1, Some(2) = Stage 2
    cache_stage: AsyncShared<Option<u8>>,
    pub editors: SharedMutex<std::collections::HashMap<u32, Editor>>,
    // Deferred initialization - store device info for lazy init
    init_device_info: SharedMutex<Option<(String, Option<wgpu::PipelineCache>, Option<u8>)>>,
    // Performance monitoring
    perf_monitor: SharedPerfMonitor,
    // Detailed diagnostics (optional, for profiling)
    #[allow(dead_code)]
    diagnostics: SharedMutex<Option<PerformanceDiagnostics>>,
    // Cached overlay editor (avoid creating every frame)
    overlay_editor: SharedMutex<Option<Editor>>,
    last_overlay_text: SharedMutex<String>,
    // Memory optimizer for tiered memory configuration
    memory_optimizer: SharedMutex<dyxel_perf::MemoryOptimizer>,
    // Async initialization state tracking
    is_loading: std::sync::Arc<std::sync::atomic::AtomicBool>,
    // Async loading thread handle (optional - for monitoring)
    #[allow(dead_code)]
    loading_handle: SharedMutex<Option<std::thread::JoinHandle<()>>>,
    // Filter pipeline for blur effects
    filter_pipeline: SharedMutex<Option<filter_pipeline::FilterPipeline>>,
    // Blurred textures to composite (cleared each frame)
    blurred_textures: SharedMutex<Vec<BlurredTextureEntry>>,
    // TextInput states for cursor and selection rendering
    text_input_states:
        SharedMutex<std::collections::HashMap<u32, dyxel_render_api::TextInputRenderState>>,
}

const BLIT_SHADER_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blit.spv"));

impl VelloBackend {
    pub fn new() -> Self {
        Self::with_perf_config(PerfConfig::default())
    }

    pub fn with_perf_config(perf_config: PerfConfig) -> Self {
        // Initialize memory optimizer with tiered configuration
        let memory_optimizer = dyxel_perf::MemoryOptimizer::new();
        log::info!(
            "[Memory] VelloBackend: Device tier detected: {:?}",
            memory_optimizer.tier()
        );

        Self {
            renderer: AsyncShared::new(std::sync::Mutex::new(None)),
            blit_bind_group_layout: SharedMutex::new(None),
            sampler: SharedMutex::new(None),
            blit_shader: SharedMutex::new(None),
            blit_pipeline: SharedMutex::new(None),
            pipeline_cache: AsyncShared::new(std::sync::Mutex::new(None)),
            cache_path: AsyncShared::new(std::sync::Mutex::new(None)),
            cache_saved: AtomicBool::new(false),
            cache_stage: AsyncShared::new(std::sync::Mutex::new(None)),
            editors: SharedMutex::new(std::collections::HashMap::new()),
            init_device_info: SharedMutex::new(None),
            perf_monitor: std::sync::Arc::new(std::sync::Mutex::new(PerformanceMonitor::new(
                perf_config,
            ))),
            diagnostics: SharedMutex::new(Some(PerformanceDiagnostics::new(120))),
            overlay_editor: SharedMutex::new(None),
            last_overlay_text: SharedMutex::new(String::new()),
            memory_optimizer: SharedMutex::new(memory_optimizer),
            is_loading: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            loading_handle: SharedMutex::new(None),
            filter_pipeline: SharedMutex::new(None),
            blurred_textures: SharedMutex::new(Vec::new()),
            text_input_states: SharedMutex::new(std::collections::HashMap::new()),
        }
    }

    /// Enable performance overlay
    pub fn enable_perf_overlay(&self) {
        self.perf_monitor.lock().unwrap().toggle_overlay();
    }

    /// Disable performance overlay
    pub fn disable_perf_overlay(&self) {
        let mut monitor = self.perf_monitor.lock().unwrap();
        if monitor.should_show_overlay() {
            monitor.toggle_overlay();
        }
    }

    /// Update TextInput state for cursor and selection rendering
    pub fn update_text_input_state(&self, node_id: u32, state: TextInputRenderState) {
        self.text_input_states
            .lock()
            .unwrap()
            .insert(node_id, state);
    }

    /// Remove TextInput state
    pub fn remove_text_input_state(&self, node_id: u32) {
        self.text_input_states.lock().unwrap().remove(&node_id);
    }

    /// Get TextInput state
    pub fn get_text_input_state(&self, node_id: u32) -> Option<TextInputRenderState> {
        self.text_input_states
            .lock()
            .unwrap()
            .get(&node_id)
            .cloned()
    }

    /// Clear all TextInput states
    pub fn clear_text_input_states(&self) {
        self.text_input_states.lock().unwrap().clear();
    }

    /// Async renderer initialization - non-blocking, runs in background thread
    /// Two-stage loading: Stage 1 (fast), save cache, Stage 2 (complete), update cache
    fn ensure_renderer_initialized_async(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Fast path - already initialized
        if self.renderer.lock().unwrap().is_some() {
            return;
        }

        // Check if already loading
        if self.is_loading.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }

        // Try to acquire init info
        let init_info = self.init_device_info.lock().unwrap().take();
        if init_info.is_none() {
            return; // No init info available (should not happen)
        }

        let (_cache_path, pipeline_cache, cache_stage) = init_info.unwrap();
        let memory_tier = self.memory_optimizer.lock().unwrap().tier();

        // Determine if we need full load based on cache stage
        // cache_stage: None = no cache, Some(1) = Stage 1 (area_only), Some(2) = Stage 2 (full)
        let needs_full_load = cache_stage != Some(2);
        let is_first_launch = cache_stage.is_none();

        log::info!(
            "[ColdStart] Cache stage: {:?}, needs_full_load: {}, is_first_launch: {}",
            cache_stage,
            needs_full_load,
            is_first_launch
        );

        // Set loading flag
        self.is_loading
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // Clone necessary data for the background thread
        let renderer_clone = self.renderer.clone();
        let is_loading_clone = self.is_loading.clone();
        let device_clone = device.clone();
        let queue_clone = queue.clone();
        let perf_monitor_clone = self.perf_monitor.clone();
        let cache_saved_clone = std::sync::Arc::new(AtomicBool::new(false));
        let cache_saved_for_thread = cache_saved_clone.clone();
        let pipeline_cache_clone = self.pipeline_cache.clone();
        let cache_path_clone: AsyncShared<Option<String>> = self.cache_path.clone();
        let cache_stage_clone = self.cache_stage.clone();

        // Spawn background thread for heavy shader compilation
        let handle = std::thread::spawn(move || {
            let start = std::time::Instant::now();

            // Determine AA support based on stage and tier
            let (aa_support, _stage_label) = if needs_full_load {
                if is_first_launch {
                    // First launch: Use area_only for fast startup
                    log::info!("[Vello] First launch: Using area_only AA for fast startup");
                    (vello::AaSupport::area_only(), "Stage 1 (first launch)")
                } else {
                    // Have Stage 1 cache, upgrading to full
                    log::info!("[Vello] Upgrading: Loading full AA support");
                    (vello::AaSupport::all(), "Stage 2 (upgrade)")
                }
            } else {
                // Have full cache
                log::info!("[Vello] Full cache hit: Using full AA support");
                (vello::AaSupport::all(), "Full cache")
            };

            // Determine thread count based on tier
            let num_threads = match memory_tier {
                dyxel_perf::DeviceMemoryTier::LowEnd => Some(2),
                dyxel_perf::DeviceMemoryTier::MidRange => Some(4),
                dyxel_perf::DeviceMemoryTier::HighEnd => {
                    std::thread::available_parallelism().ok().map(|n| n.get())
                }
            };

            let options = RendererOptions {
                antialiasing_support: aa_support,
                pipeline_cache,
                num_init_threads: num_threads.and_then(|n| std::num::NonZeroUsize::new(n)),
                use_cpu: false,
            };

            // Stage 1: Create renderer with appropriate AA mode
            let renderer_result = Renderer::new(&device_clone, options);

            match renderer_result {
                Ok(mut renderer) => {
                    log::info!(
                        "[ColdStart] Renderer::new() completed in {:?}",
                        start.elapsed()
                    );

                    // Perform minimal warmup
                    let warmup_start = std::time::Instant::now();
                    let dummy_texture = device_clone.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Async Warmup Texture"),
                        size: wgpu::Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::STORAGE_BINDING,
                        view_formats: &[],
                    });
                    let dummy_view =
                        dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());
                    let scene = Scene::new();
                    let params = vello::RenderParams {
                        base_color: Color::TRANSPARENT,
                        width: 1,
                        height: 1,
                        antialiasing_method: vello::AaConfig::Area,
                    };
                    let _ = renderer.render_to_texture(
                        &device_clone,
                        &queue_clone,
                        &scene,
                        &dummy_view,
                        &params,
                    );
                    log::info!(
                        "[ColdStart] Warmup completed in {:?}",
                        warmup_start.elapsed()
                    );

                    // Store renderer
                    *renderer_clone.lock().unwrap() = Some(renderer);

                    // Save Stage 1 cache only if we needed full load (first launch or Stage 1 upgrade)
                    // If we already had Stage 2 cache (needs_full_load=false), no need to save
                    if needs_full_load {
                        log::info!("[ColdStart] Saving Stage 1 cache");

                        let cache_lock = pipeline_cache_clone.lock().unwrap();
                        let path_lock = cache_path_clone.lock().unwrap();
                        if let (Some(cache), Some(path)) = (&*cache_lock, &*path_lock) {
                            if let Some(data) = cache.get_data() {
                                // Add header to mark as Stage 1
                                let mut cache_with_header = Vec::with_capacity(data.len() + 1);
                                cache_with_header.push(1u8); // Stage 1 marker
                                cache_with_header.extend_from_slice(&data);

                                if std::fs::write(path, &cache_with_header).is_ok() {
                                    cache_saved_for_thread
                                        .store(true, std::sync::atomic::Ordering::SeqCst);
                                    *cache_stage_clone.lock().unwrap() = Some(1);
                                    log::info!(
                                        "[ColdStart] Stage 1 cache saved ({} bytes)",
                                        cache_with_header.len()
                                    );
                                }
                            }
                        }
                        drop(cache_lock);
                        drop(path_lock);
                    }

                    // Stage 2: If this is Stage 1 (first launch with area_only), upgrade to full in background
                    if is_first_launch && memory_tier != dyxel_perf::DeviceMemoryTier::LowEnd {
                        log::info!("[ColdStart] Starting Stage 2: Upgrading to full AA support in background");

                        let stage2_start = std::time::Instant::now();
                        let full_options = RendererOptions {
                            antialiasing_support: vello::AaSupport::all(),
                            pipeline_cache: pipeline_cache_clone.lock().unwrap().clone(),
                            num_init_threads: num_threads
                                .and_then(|n| std::num::NonZeroUsize::new(n)),
                            use_cpu: false,
                        };

                        // Try to create full renderer (will reuse Stage 1 cache + compile remaining)
                        match Renderer::new(&device_clone, full_options) {
                            Ok(full_renderer) => {
                                log::info!(
                                    "[ColdStart] Stage 2 complete in {:?}",
                                    stage2_start.elapsed()
                                );

                                // Replace the Stage 1 renderer with full renderer
                                *renderer_clone.lock().unwrap() = Some(full_renderer);

                                // Save Stage 2 cache

                                let cache_lock = pipeline_cache_clone.lock().unwrap();
                                let path_lock = cache_path_clone.lock().unwrap();
                                if let (Some(cache), Some(path)) = (&*cache_lock, &*path_lock) {
                                    if let Some(data) = cache.get_data() {
                                        let mut cache_with_header =
                                            Vec::with_capacity(data.len() + 1);
                                        cache_with_header.push(2u8); // Stage 2 marker (full)
                                        cache_with_header.extend_from_slice(&data);

                                        if std::fs::write(path, &cache_with_header).is_ok() {
                                            log::info!(
                                                "[ColdStart] Stage 2 cache saved ({} bytes)",
                                                cache_with_header.len()
                                            );
                                            // Update cache_stage to Stage 2
                                            *cache_stage_clone.lock().unwrap() = Some(2);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                log::warn!(
                                    "[ColdStart] Stage 2 failed: {}, keeping Stage 1 renderer",
                                    e
                                );
                            }
                        }
                    }

                    // Record startup performance (Stage 1 time)
                    perf_monitor_clone
                        .lock()
                        .unwrap()
                        .record_startup_time(start.elapsed());
                }
                Err(e) => {
                    log::error!("[ColdStart] Failed to create renderer: {}", e);
                }
            }

            is_loading_clone.store(false, std::sync::atomic::Ordering::SeqCst);
        });

        *self.loading_handle.lock().unwrap() = Some(handle);
    }

    /// Check if renderer is ready for rendering
    pub fn is_renderer_ready(&self) -> bool {
        self.renderer.lock().unwrap().is_some()
    }

    /// Check if renderer is currently loading
    pub fn is_renderer_loading(&self) -> bool {
        self.is_loading.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn save_cache(&self) {
        if self.cache_saved.load(std::sync::atomic::Ordering::SeqCst) {
            log::debug!("[ColdStart] Cache already saved, skipping");
            return;
        }
        let cache_lock = self.pipeline_cache.lock().unwrap();
        let path_lock = self.cache_path.lock().unwrap();
        let stage_lock = self.cache_stage.lock().unwrap();
        if let (Some(cache), Some(path)) = (&*cache_lock, &*path_lock) {
            #[cfg(not(target_arch = "wasm32"))]
            {
                log::info!("[ColdStart] Saving pipeline cache to: {}", path);
                if let Some(data) = cache.get_data() {
                    log::info!("[ColdStart] Cache data size: {} bytes", data.len());

                    // Add stage header if we have a valid stage
                    let result = if let Some(stage) = *stage_lock {
                        if stage == 1 || stage == 2 {
                            let mut cache_with_header = Vec::with_capacity(data.len() + 1);
                            cache_with_header.push(stage);
                            cache_with_header.extend_from_slice(&data);
                            log::info!("[ColdStart] Saving with Stage {} header", stage);
                            std::fs::write(path, &cache_with_header)
                        } else {
                            std::fs::write(path, &data)
                        }
                    } else {
                        std::fs::write(path, &data)
                    };

                    if let Err(e) = result {
                        log::error!("[ColdStart] Failed to save pipeline cache: {}", e);
                    } else {
                        log::info!(
                            "[ColdStart] Pipeline cache saved successfully ({} bytes)",
                            data.len()
                        );
                        self.cache_saved
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                } else {
                    log::warn!("[ColdStart] Cache get_data() returned None");
                }
            }
            #[cfg(target_arch = "wasm32")]
            let _ = (cache, path);
        } else {
            log::warn!(
                "[ColdStart] Cannot save cache: cache={}, path={}",
                cache_lock.is_some(),
                path_lock.is_some()
            );
        }
    }

    /// Prewarm pipelines: create all necessary pipelines in background to reduce first-render latency
    fn prewarm_pipelines(&self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        log::info!("VelloBackend: Prewarming pipelines...");
        let blit_shader = self.blit_shader.lock().unwrap();
        let blit_layout = self.blit_bind_group_layout.lock().unwrap();

        if let (Some(shader), Some(layout)) = (&*blit_shader, &*blit_layout) {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Blit Pipeline Layout Prewarm"),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Blit Pipeline Prewarm"),
                layout: Some(&pl),
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
                cache: self.pipeline_cache.lock().unwrap().as_ref(),
            });
            *self.blit_pipeline.lock().unwrap() = Some(pipeline);
        }
        log::info!("VelloBackend: Pipeline prewarming complete.");
    }

    /// Clear surface with a simple color (fallback when renderer is loading)
    fn clear_surface(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        v_surface_surface: &mut vello::util::RenderSurface<'static>,
    ) -> RenderResult {
        // Get current texture
        let surface_texture = match v_surface_surface.surface.get_current_texture() {
            Ok(st) => st,
            Err(e) => {
                log::warn!("[ClearSurface] Failed to get current texture: {:?}", e);
                return Ok(());
            }
        };

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Clear Surface (Async Loading)"),
        });

        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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

        queue.submit(Some(encoder.finish()));
        surface_texture.present();

        Ok(())
    }

    fn render_internal(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        v_surface_surface: &mut vello::util::RenderSurface<'static>,
        blit_pipeline: &wgpu::RenderPipeline,
        offscreen_texture: &mut Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
        shared_state: &SharedMutex<SharedState>,
    ) -> RenderResult {
        // Detailed frame timing for diagnostics
        #[cfg(not(target_os = "android"))]
        let frame_start = std::time::Instant::now();
        let mut stage_timer = dyxel_perf::FrameTimer::new();

        // Async initialization: start background compilation without blocking
        self.ensure_renderer_initialized_async(device, queue);
        stage_timer.mark("init_check");

        // Check if renderer is ready
        let mut renderer_lock = self.renderer.lock().unwrap();
        let renderer = match renderer_lock.as_mut() {
            Some(r) => r,
            None => {
                // Renderer not ready yet - clear surface and return
                // This keeps the main loop at 60fps while shader compiles in background
                drop(renderer_lock); // Release lock before calling clear_surface
                return self.clear_surface(device, queue, v_surface_surface);
            }
        };

        // Begin frame timing for performance monitoring
        let should_show_overlay = {
            let monitor = self.perf_monitor.lock().unwrap();
            monitor.begin_frame();
            monitor.should_show_overlay()
        };
        stage_timer.mark("perf_start");

        let w = v_surface_surface.config.width;
        let h = v_surface_surface.config.height;
        if w == 0 || h == 0 {
            return Ok(());
        }

        // Get or create editors for text nodes and compute layout
        let rid = {
            let mut g = shared_state.lock().unwrap();
            let mut editors = self.editors.lock().unwrap();

            // Get text input states for Input nodes
            let text_input_states = get_all_text_input_states();

            // First pass: create/update editors for text and input nodes
            for (&id, node) in &g.nodes {
                if node.view_type == ViewType::Text {
                    let editor = editors.entry(id).or_insert_with(|| {
                        let mut ed = Editor::new(node.font_size);
                        ed.set_text(&node.text);
                        ed.set_text_color(node.color);
                        ed
                    });

                    // Update editor if text changed
                    if editor.text() != node.text {
                        editor.set_text(&node.text);
                    }
                } else if node.view_type == ViewType::Input {
                    // For TextInput nodes, get text from text_input_states
                    let text_input_text = text_input_states
                        .get(&id)
                        .map(|s| s.text.clone())
                        .unwrap_or_default();

                    log::debug!("Creating editor for Input node {}, text='{}', font_size={}", id, text_input_text, node.font_size);

                    let editor = editors.entry(id).or_insert_with(|| {
                        log::debug!("Inserting new editor for node {}", id);
                        let mut ed = Editor::new(node.font_size);
                        ed.set_text(&text_input_text);
                        ed.set_text_color(node.color);
                        ed
                    });

                    // Update editor if text changed
                    if editor.text() != text_input_text {
                        editor.set_text(&text_input_text);
                    }
                }
            }

            // Remove editors for deleted nodes
            let node_ids: std::collections::HashSet<u32> = g.nodes.keys().copied().collect();
            editors.retain(|id, _| node_ids.contains(id));

            // Build map from taffy_node to editor id for measurement
            let taffy_to_id: std::collections::HashMap<taffy::NodeId, u32> = g
                .nodes
                .iter()
                .filter(|(_, n)| n.view_type == ViewType::Text)
                .map(|(id, n)| (n.taffy_node, *id))
                .collect();

            // Second pass: measure text nodes and detect size changes
            // Collect nodes whose size changed significantly
            let mut nodes_to_update: Vec<(u32, f32, f32)> = Vec::new();
            for (&id, node) in &g.nodes {
                if node.view_type == ViewType::Text {
                    if let Some(editor) = editors.get_mut(&id) {
                        editor.set_width(None);
                        let (new_width, new_height) = editor.layout_size();
                        let (old_width, old_height) = node.last_measured_size;

                        // If size changed significantly (more than 0.5px), record for update
                        if (new_width - old_width).abs() > 0.5
                            || (new_height - old_height).abs() > 0.5
                        {
                            nodes_to_update.push((id, new_width, new_height));
                        }
                    }
                }
            }

            // Update last_measured_size and mark dirty (triggers Taffy relayout via set_style)
            for (id, new_width, new_height) in nodes_to_update {
                if let Some(node_mut) = g.nodes.get_mut(&id) {
                    node_mut.last_measured_size = (new_width, new_height);
                }
                g.mark_dirty(id);
            }

            let rid = g.root_id.map(|id| {
                if let Some(rn) = g.nodes.get(&id).map(|n| n.taffy_node) {
                    let _ = g.taffy.compute_layout_with_measure(
                        rn,
                        taffy::prelude::Size {
                            width: AvailableSpace::Definite(w as f32),
                            height: AvailableSpace::Definite(h as f32),
                        },
                        |_known_dimensions, _available_space, node_id, _node_context, _style| {
                            // Look up editor by taffy_node
                            if let Some(&editor_id) = taffy_to_id.get(&node_id) {
                                if let Some(editor) = editors.get_mut(&editor_id) {
                                    // For text nodes: always use natural width (no wrapping)
                                    // This prevents unwanted wrapping from parent flex constraints
                                    // In the future, we could respect explicit width settings here
                                    editor.set_width(None);
                                    let (lw, lh) = editor.layout_size();
                                    return taffy::geometry::Size {
                                        width: lw,
                                        height: lh,
                                    };
                                }
                            }
                            // Not a text node, return default
                            taffy::geometry::Size {
                                width: _known_dimensions.width.unwrap_or(0.0),
                                height: _known_dimensions.height.unwrap_or(0.0),
                            }
                        },
                    );

                    // Register all nodes as layout-dirty after computation
                    // This ensures Logic Thread will sync layout to WASM memory
                    {
                        let node_ids: Vec<u32> = g.nodes.keys().copied().collect();
                        dyxel_shared::layout_sync::register_layout_dirty_nodes(&node_ids);
                    }

                    // Sync layout results and generations to SharedBuffer (for WASM/Guest access)
                    // This replaces the old sync_layout_to_wasm function
                    g.sync_to_shared_buffer();

                    // Phase 2: Auto-expand capacity if needed (pre-expand at 80% usage)
                    if g.should_pre_expand() {
                        if g.auto_expand() {
                            log::info!("Auto-expanded node capacity to {}", g.get_capacity());
                        }
                    }

                    // 每 300 帧（约 5 秒 @ 60fps）输出一次节点统计
                    #[cfg(target_os = "android")]
                    {
                        static mut FRAME_COUNTER: u32 = 0;
                        unsafe {
                            FRAME_COUNTER += 1;
                            if FRAME_COUNTER % 300 == 0 {
                                let stats = g.get_stats();
                                log::info!(
                                    "[NodeStats] capacity={} active={} free={} usage={:.1}%",
                                    stats.capacity,
                                    stats.active_count,
                                    stats.free_count,
                                    (stats.active_count as f32 / stats.capacity as f32) * 100.0
                                );
                            }
                        }
                    }
                }
                id
            });

            rid
        };

        let mut scene = Scene::new();

        if let Some(id) = rid {
            let g = shared_state.lock().unwrap();
            let mut editors = self.editors.lock().unwrap();
            stage_timer.mark("state_lock");

            // Apply platform correction at the root level
            let platform_transform = platform_correction(h as f64);

            // Apply keyboard avoidance offset (negative Y shifts content up)
            let keyboard_offset = crate::keyboard::keyboard_offset() as f64;
            let root_transform = platform_transform * Affine::translate((0.0, keyboard_offset));

            // Get filter pipeline for blur effects
            let filter_pipeline = self.filter_pipeline.lock().unwrap();
            let mut blurred_textures = self.blurred_textures.lock().unwrap();

            // Get global text input states
            let text_input_states = get_all_text_input_states();

            render_node_recursive_with_transform(
                id,
                &g,
                &mut editors,
                &mut scene,
                Vec2::ZERO,
                root_transform,
                device,
                queue,
                renderer,
                filter_pipeline.as_ref(),
                &mut blurred_textures,
                text_input_states,
            );
            stage_timer.mark("scene_build");
        }

        // Get performance stats and draw overlay directly to scene if enabled
        let stats = self.perf_monitor.lock().unwrap().get_stats();
        if should_show_overlay {
            let overlay_text = format!(
                "FPS: {:.1}\nFrame: {:.2}ms\nMem: {:.1}MB\nCPU: {:.1}%",
                stats.fps, stats.frame_time_ms, stats.memory_used_mb, stats.cpu_usage
            );

            // Calculate overlay position (top-left corner with padding)
            let (overlay_x, overlay_y, _) = self.perf_monitor.lock().unwrap().get_overlay_config();
            let padding = 10.0;
            let pos_x = padding + overlay_x as f64;
            let pos_y = padding + overlay_y as f64;

            // Draw semi-transparent background directly to main scene
            let bg_rect = KRect::new(pos_x - 5.0, pos_y - 5.0, pos_x + 140.0, pos_y + 70.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgba8(0, 0, 0, 180),
                None,
                &bg_rect,
            );

            // Use cached editor (avoid creating every frame)
            let mut editor_lock = self.overlay_editor.lock().unwrap();
            let mut last_text_lock = self.last_overlay_text.lock().unwrap();

            if editor_lock.is_none() {
                *editor_lock = Some(Editor::new(14.0));
            }

            if let Some(ref mut editor) = *editor_lock {
                // Only update text if changed (avoid expensive re-layout)
                if *last_text_lock != overlay_text {
                    editor.set_text(&overlay_text);
                    editor.set_text_color(Color::WHITE);
                    *last_text_lock = overlay_text;
                }

                // Draw text directly to main scene using cached editor
                editor.draw(&mut scene, Affine::translate((pos_x, pos_y)));
            }
        }

        // Offscreen logic alignment - Vello requires Rgba8Unorm for storage textures
        if offscreen_texture
            .as_ref()
            .map_or(true, |(t, _, _)| t.width() != w || t.height() != h)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Vello Offscreen Texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Vello Blit Bind Group"),
                layout: self
                    .blit_bind_group_layout
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap(),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(
                            self.sampler.lock().unwrap().as_ref().unwrap(),
                        ),
                    },
                ],
            });
            *offscreen_texture = Some((texture, view, bg));
        }

        let (_, off_view, blit_bg) = offscreen_texture.as_ref().unwrap();

        // Tier-based AA configuration: reduce quality for LowEnd to save memory
        let multiplier = self
            .memory_optimizer
            .lock()
            .unwrap()
            .vello_buffer_multiplier();
        let aa_config = if multiplier < 0.5 {
            vello::AaConfig::Area // LowEnd: use simpler AA
        } else {
            vello::AaConfig::Area // Default to Area for consistent performance
        };

        // Single render: main scene + overlay (if enabled) to offscreen texture
        renderer
            .render_to_texture(
                device,
                queue,
                &scene,
                off_view,
                &vello::RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: w,
                    height: h,
                    antialiasing_method: aa_config,
                },
            )
            .map_err(|e| anyhow::anyhow!("Vello render error: {:?}", e))?;
        stage_timer.mark("gpu_render");

        // Single present: blit the combined result (main scene + optional overlay) to screen
        match v_surface_surface.surface.get_current_texture() {
            Ok(st) => {
                let mut enc = device.create_command_encoder(&Default::default());
                {
                    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Vello Blit Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &st.texture.create_view(&Default::default()),
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    rp.set_pipeline(blit_pipeline);
                    rp.set_bind_group(0, blit_bg, &[]);
                    rp.draw(0..3, 0..1);

                    // Draw blurred textures - TEMPORARILY DISABLED for debugging
                    let blurred_textures = self.blurred_textures.lock().unwrap();
                    if !blurred_textures.is_empty() {
                        log::debug!(
                            "[Blur] Have {} blurred textures to draw (disabled for debugging)",
                            blurred_textures.len()
                        );
                        // TODO: Re-enable blur texture drawing after fixing black screen
                    }
                    // Clear blurred textures after drawing
                    drop(blurred_textures);
                    self.blurred_textures.lock().unwrap().clear();
                }
                queue.submit(Some(enc.finish()));
                stage_timer.mark("blit_submit");
                st.present();
                stage_timer.mark("present_return");

                // After first successful render, save the pipeline cache
                // This ensures cache is complete with all compiled shaders
                static FIRST_RENDER_DONE: std::sync::atomic::AtomicBool =
                    std::sync::atomic::AtomicBool::new(false);
                if !FIRST_RENDER_DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    log::info!("[ColdStart] First render completed, saving pipeline cache");
                    self.save_cache();
                }
            }
            Err(e) => {
                log::error!("VelloBackend: get_current_texture failed: {:?}", e);
                return Err(anyhow::anyhow!(
                    "Surface texture acquisition failed: {:?}",
                    e
                ));
            }
        }

        // Log detailed frame timing every 60 frames for diagnostics
        let stats = self.perf_monitor.lock().unwrap().get_stats();
        if stats.total_frames % 60 == 0 {
            let report = stage_timer.report();

            // Calculate stage durations
            #[cfg(not(target_os = "android"))]
            let state_lock_time =
                report.get("init_done_to_perf_start") + report.get("perf_start_to_state_lock");
            #[cfg(not(target_os = "android"))]
            let scene_build_time = report.get("state_lock_to_scene_build");
            #[cfg(not(target_os = "android"))]
            let gpu_time = report.get("scene_build_to_gpu_render");
            #[cfg(not(target_os = "android"))]
            let blit_time = report.get("gpu_render_to_blit_submit");
            #[cfg(not(target_os = "android"))]
            let present_time = report.get("blit_submit_to_present_return");
            #[cfg(not(target_os = "android"))]
            let total = frame_start.elapsed().as_secs_f32() * 1000.0;

            #[cfg(target_os = "android")]
            {
                let perf_monitor = self.perf_monitor.lock().unwrap();
                let _mem_trend = perf_monitor.get_memory_trend();
                let _leak_warning = if perf_monitor.has_memory_leak() {
                    " [LEAK]"
                } else {
                    ""
                };
                drop(perf_monitor);

                // Temperature and thermal status
                let _temp_str = if let Some(temp) = stats.temperature_c {
                    let thermal_status = if temp > 75.0 {
                        "🔥 THROTTLING"
                    } else if temp > 60.0 {
                        "⚠️  WARM"
                    } else {
                        "✓ OK"
                    };
                    format!(", Temp={:.1}°C {}", temp, thermal_status)
                } else {
                    String::new()
                };

                // NOTE: Frame diagnostic logging disabled for cleaner logs
                // log::info!(
                //     "[DIAG-Android] Frame {}: {:.2}ms (State={:.2} Scene={:.2} GPU={:.2} Blit={:.2} Present={:.2}) FPS={:.1} Mem={:.1}MB ({:.1}/min){}{}",
                //     ...
                // );
            }

            #[cfg(not(target_os = "android"))]
            log::debug!(
                "[DIAG] Frame {}: Total={:.2}ms, State={:.2}ms, Scene={:.2}ms, GPU={:.2}ms, Blit={:.2}ms, Present={:.2}ms, FPS={:.1}",
                stats.total_frames,
                total,
                state_lock_time,
                scene_build_time,
                gpu_time,
                blit_time,
                present_time,
                stats.fps
            );

            // Print full breakdown every 300 frames (5 seconds at 60 FPS)
            // Note: Only printed when debug logging is enabled
            if stats.total_frames % 300 == 0 && log::log_enabled!(log::Level::Debug) {
                report.print();
            }
        }

        Ok(())
    }
}

// =============================================================================
// Platform Coordinate System Correction
// =============================================================================

/// Returns the platform-specific coordinate correction transform.
#[inline]
pub fn platform_correction(viewport_height: f64) -> Affine {
    #[cfg(target_os = "android")]
    {
        Affine::translate((0.0, viewport_height)) * Affine::scale_non_uniform(1.0, -1.0)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = viewport_height;
        Affine::IDENTITY
    }
}

/// Render a node with layer effects (alpha, blur, shadow, clip)
/// Render node content with blur effect applied
///
/// This creates an offscreen texture, renders the node content to it,
/// applies blur using compute shaders, and draws the result to the main scene.
fn render_with_blur(
    node: &dyxel_shared::ViewNode,
    id: u32,
    state: &SharedState,
    editors: &mut std::collections::HashMap<u32, Editor>,
    scene: &mut Scene,
    local_transform: Affine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    filter_pipeline: &crate::filter_pipeline::FilterPipeline,
    node_width: f64,
    node_height: f64,
    _needs_layer: bool,
    blurred_textures: &mut Vec<BlurredTextureEntry>,
    text_input_states: std::collections::HashMap<u32, dyxel_render_api::TextInputRenderState>,
) -> bool {
    use kurbo::{Rect as KRect, RoundedRect};
    use vello::peniko::{Color, Fill};

    // Calculate padded size for blur (need extra space for blur bleed)
    let blur_radius = node.blur_radius as f64;
    let padding = (blur_radius * 2.5).ceil() as u32;
    let texture_width = (node_width as u32 + padding * 2).max(1);
    let texture_height = (node_height as u32 + padding * 2).max(1);

    // Create offscreen texture for rendering
    let texture_desc = wgpu::TextureDescriptor {
        label: Some("Blur Offscreen Texture"),
        size: wgpu::Extent3d {
            width: texture_width,
            height: texture_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    };

    let offscreen_texture = device.create_texture(&texture_desc);
    let texture_view = offscreen_texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Create intermediate texture for ping-pong blur
    let intermediate_desc = wgpu::TextureDescriptor {
        label: Some("Blur Intermediate Texture"),
        size: wgpu::Extent3d {
            width: texture_width / 4,
            height: texture_height / 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    };

    let _intermediate_texture = device.create_texture(&intermediate_desc);

    // Create a temporary scene for this node and its children
    let mut temp_scene = Scene::new();

    // Adjust transform to account for padding
    let offset_transform = Affine::translate((padding as f64, padding as f64));

    // Render background rectangle
    let rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
    if node.border_radius > 0.0 {
        let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
        temp_scene.fill(Fill::NonZero, offset_transform, node.color, None, &rounded);
    } else {
        temp_scene.fill(Fill::NonZero, offset_transform, node.color, None, &rect);
    }

    // Render text if present
    if node.view_type == ViewType::Text {
        if let Some(editor) = editors.get_mut(&id) {
            editor.set_width(None);
            editor.draw(&mut temp_scene, offset_transform);
        }
    }

    // Render children to temp scene (with adjusted positions)
    // Note: We need to offset children by padding
    for &child_id in &node.children {
        render_child_to_blur_scene(
            child_id,
            state,
            editors,
            &mut temp_scene,
            offset_transform,
            padding as f64,
        );
    }

    // Note: Layer popping is now handled in the main render function
    // since blur texture compositing is disabled
    // TODO: Re-enable this after fixing blur compositing
    // if needs_layer {
    //     scene.pop_layer();
    // }

    // Render temp scene to offscreen texture
    let render_params = vello::RenderParams {
        base_color: Color::TRANSPARENT,
        width: texture_width,
        height: texture_height,
        antialiasing_method: vello::AaConfig::Area,
    };

    log::debug!(
        "[Blur] Rendering to offscreen texture {}x{}",
        texture_width,
        texture_height
    );
    if let Err(e) =
        renderer.render_to_texture(device, queue, &temp_scene, &texture_view, &render_params)
    {
        log::warn!("[Blur] Failed to render to offscreen texture: {:?}", e);
        return false;
    }
    log::debug!("[Blur] Offscreen render complete");

    // Apply blur using filter pipeline
    if let Err(e) =
        filter_pipeline.apply_blur(&offscreen_texture, &offscreen_texture, node.blur_radius)
    {
        log::warn!("[Blur] Failed to apply blur: {:?}", e);
        return false;
    }

    // Store the blurred texture for compositing in the final blit pass
    // Adjust transform to account for the padding offset
    let final_transform =
        local_transform * Affine::translate((-(padding as f64), -(padding as f64)));

    blurred_textures.push(BlurredTextureEntry {
        texture: offscreen_texture,
        width: texture_width,
        height: texture_height,
        transform: final_transform,
        opacity: node.opacity,
    });

    // Draw children that extend beyond bounds (if not clipped)
    if !node.clip_to_bounds {
        let local_pos = Vec2::new(node_width / 2.0, node_height / 2.0);
        for &child_id in &node.children {
            render_node_recursive_with_transform(
                child_id,
                state,
                editors,
                scene,
                local_pos,
                local_transform,
                device,
                queue,
                renderer,
                Some(filter_pipeline),
                blurred_textures,
                text_input_states.clone(),
            );
        }
    }

    true
}

/// Helper to render a child node to the blur temp scene
fn render_child_to_blur_scene(
    id: u32,
    state: &SharedState,
    editors: &mut std::collections::HashMap<u32, Editor>,
    scene: &mut Scene,
    transform: Affine,
    padding_offset: f64,
) {
    use kurbo::{Rect as KRect, RoundedRect};
    use vello::peniko::Fill;

    if let Some(node) = state.nodes.get(&id) {
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let x = layout.location.x as f64 + node.position_x as f64 + padding_offset;
        let y = layout.location.y as f64 + node.position_y as f64 + padding_offset;
        let width = layout.size.width as f64;
        let height = layout.size.height as f64;

        let local_transform = transform * Affine::translate((x, y));

        // Draw the child
        let rect = KRect::from_origin_size((0.0, 0.0), (width, height));
        if node.border_radius > 0.0 {
            let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
            scene.fill(Fill::NonZero, local_transform, node.color, None, &rounded);
        } else {
            scene.fill(Fill::NonZero, local_transform, node.color, None, &rect);
        }

        // Recursively render grandchildren
        for &child_id in &node.children {
            render_child_to_blur_scene(child_id, state, editors, scene, local_transform, 0.0);
        }
    }
}

/// Render a node with layer effects (alpha, blur, shadow, clip)
/// Following Xilem's pattern: shadow -> content -> children
fn render_node_recursive_with_transform(
    id: u32,
    state: &SharedState,
    editors: &mut std::collections::HashMap<u32, Editor>,
    scene: &mut Scene,
    parent_pos: Vec2,
    transform: Affine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    filter_pipeline: Option<&crate::filter_pipeline::FilterPipeline>,
    blurred_textures: &mut Vec<BlurredTextureEntry>,
    text_input_states: std::collections::HashMap<u32, dyxel_render_api::TextInputRenderState>,
) {
    use kurbo::{Affine, Rect as KRect, RoundedRect};
    use vello::peniko::{BlendMode as PenikoBlendMode, Compose, Fill, Mix};

    if let Some(node) = state.nodes.get(&id) {
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let taffy_x = layout.location.x as f64;
        let taffy_y = layout.location.y as f64;
        let node_width = layout.size.width as f64;
        let node_height = layout.size.height as f64;
        let global_pos = parent_pos + Vec2::new(taffy_x, taffy_y);

        // Build local transform for this node
        // Apply position offset if set (for absolute positioning within parent)
        let pos_offset = Vec2::new(node.position_x as f64, node.position_y as f64);
        let local_transform = transform
            * Affine::translate((global_pos.x + pos_offset.x, global_pos.y + pos_offset.y));

        // Determine if we need layer effects
        let needs_layer = node.opacity < 1.0 || node.clip_to_bounds || node.blur_radius > 0.0;
        let has_shadow = node.shadow_blur > 0.0
            && (node.shadow_offset_x != 0.0
                || node.shadow_offset_y != 0.0
                || node.shadow_blur > 0.0);

        // === Step 1: Draw Shadow (if any, using blur) ===
        // Xilem pattern: Draw shadow first, then content on top
        if has_shadow {
            let shadow_x = node.shadow_offset_x as f64;
            let shadow_y = node.shadow_offset_y as f64;
            let blur_radius = node.shadow_blur as f64;

            // Extract shadow color components
            let r = ((node.shadow_color >> 16) & 0xFF) as u8;
            let g = ((node.shadow_color >> 8) & 0xFF) as u8;
            let b = (node.shadow_color & 0xFF) as u8;
            let a = ((node.shadow_color >> 24) & 0xFF) as u8;
            let shadow_color = vello::peniko::Color::from_rgba8(r, g, b, a);

            // Draw blurred shadow using Vello's draw_blurred_rounded_rect
            let rect = KRect::from_origin_size((shadow_x, shadow_y), (node_width, node_height));

            if node.border_radius > 0.0 {
                scene.draw_blurred_rounded_rect(
                    local_transform,
                    rect,
                    shadow_color,
                    node.border_radius as f64,
                    blur_radius,
                );
            } else {
                scene.draw_blurred_rounded_rect(
                    local_transform,
                    rect,
                    shadow_color,
                    0.0,
                    blur_radius,
                );
            }
        }

        // === Step 2: Push Layer (if needed for alpha/blur/clip) ===
        if needs_layer {
            // Convert opacity to layer alpha
            let alpha = node.opacity.clamp(0.0, 1.0);

            // Default blend mode (Normal)
            let blend = PenikoBlendMode::new(Mix::Normal, Compose::SrcOver);

            // Create clip shape if clip_to_bounds is enabled
            if node.clip_to_bounds {
                // Use rounded rect clip if border_radius is set
                if node.border_radius > 0.0 {
                    let clip_rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
                    let rounded_clip = RoundedRect::from_rect(clip_rect, node.border_radius as f64);
                    scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &rounded_clip);
                } else {
                    let clip_rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
                    scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &clip_rect);
                }
            } else {
                // No clipping - use large rect
                let full_rect = KRect::from_origin_size((-1e6, -1e6), (2e6, 2e6));
                scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &full_rect);
            }
        }

        // === Step 3: Handle Blur Effect ===
        // If blur is enabled, render to offscreen texture and apply blur
        let has_blur = node.blur_radius > 0.0;
        let _blur_applied = if has_blur && filter_pipeline.is_some() {
            render_with_blur(
                node,
                id,
                state,
                editors,
                scene,
                local_transform,
                device,
                queue,
                renderer,
                filter_pipeline.unwrap(),
                node_width,
                node_height,
                needs_layer,
                blurred_textures,
                text_input_states.clone(),
            )
        } else {
            false
        };

        // === Step 4: Draw Node Content ===
        // Note: Always draw content for now since blur texture compositing is disabled
        // TODO: Re-enable !blur_applied check after fixing blur compositing
        if true {
            if node.view_type == ViewType::Text || node.view_type == ViewType::Input {
                // For Input nodes, render background and border first
                if node.view_type == ViewType::Input {
                    let rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
                    if node.border_radius > 0.0 {
                        let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                        scene.fill(Fill::NonZero, local_transform, node.color, None, &rounded);
                    } else {
                        scene.fill(Fill::NonZero, local_transform, node.color, None, &rect);
                    }
                }

                // Render text using Editor
                log::debug!("Input node {}: looking for editor, editors.len()={}", id, editors.len());
                if let Some(editor) = editors.get_mut(&id) {
                    log::debug!("Input node {}: found editor with text '{}'", id, editor.text());
                    // Get text layout size for alignment calculation
                    let (text_width, _) = editor.layout_size();
                    let available_width = node_width as f32;

                    // Calculate x offset based on text alignment (horizontal)
                    let x_offset = match node.text_align {
                        dyxel_shared::TextAlign::Start => 0.0f64,
                        dyxel_shared::TextAlign::Center => {
                            ((available_width - text_width) / 2.0).max(0.0) as f64
                        }
                        dyxel_shared::TextAlign::End => {
                            (available_width - text_width).max(0.0) as f64
                        }
                        dyxel_shared::TextAlign::Justified => 0.0f64, // TODO: implement justified
                    };

                    // Calculate y offset based on vertical alignment
                    let (_, text_height) = editor.layout_size();
                    let available_height = node_height as f32;
                    let y_offset = match node.vertical_align {
                        dyxel_shared::VerticalAlign::Top => 0.0f64,
                        dyxel_shared::VerticalAlign::Center => {
                            ((available_height - text_height) / 2.0).max(0.0) as f64
                        }
                        dyxel_shared::VerticalAlign::Bottom => {
                            (available_height - text_height).max(0.0) as f64
                        }
                    };

                    // Apply alignment offset to transform
                    let align_transform = local_transform * Affine::translate((x_offset, y_offset));

                    // === Handle TextInput rendering ===
                    log::debug!("Input node {}: text_input_states.len()={}, looking for state", id, text_input_states.len());
                    if let Some(text_input) = text_input_states.get(&id) {
                        log::debug!("Input node {}: found text_input state, focused={}, text_len={}, placeholder_len={}",
                            id, text_input.focused, text_input.text.len(), text_input.placeholder.len());
                        // Render focus border when focused
                        if text_input.focused {
                            let layout_size = state
                                .taffy
                                .layout(node.taffy_node)
                                .map(|l| (l.size.width as f64, l.size.height as f64))
                                .unwrap_or((node_width, node_height));
                            render_focus_border(
                                scene,
                                local_transform,
                                0.0,
                                0.0,
                                layout_size.0,
                                layout_size.1,
                                2.0, // border width
                                Color::from_rgb8(0, 122, 255), // iOS blue
                                node.border_radius as f64,
                            );
                        }

                        // Password mode: render dots instead of actual text
                        if text_input.secure && !editor.text().is_empty() {
                            let mut secure_editor = dyxel_editor::Editor::new(node.font_size);
                            let dot_text = "●".repeat(editor.text().chars().count());
                            secure_editor.set_text(&dot_text);
                            secure_editor.set_text_color(node.color);
                            secure_editor.draw(scene, align_transform);
                        } else if editor.text().is_empty()
                            && !text_input.focused
                            && !text_input.placeholder.is_empty()
                        {
                            let mut placeholder_editor = dyxel_editor::Editor::new(node.font_size);
                            placeholder_editor.set_text(&text_input.placeholder);
                            placeholder_editor
                                .set_text_color(Color::from_rgba8(102, 102, 102, 204));
                            placeholder_editor.draw(scene, align_transform);
                        } else {
                            // Set text color before drawing (editor was created with node.color which is background color)
                            editor.set_text_color(Color::from_rgba8(0, 0, 0, 255)); // Default to black text
                            editor.draw(scene, align_transform);
                        }

                        // === Render selection highlight ===
                        if text_input.focused && text_input.selection_start != text_input.cursor_pos
                        {
                            let (text_width, text_height) = editor.layout_size();
                            let text_len = editor.text().len().max(1);
                            let start_frac = (text_input.selection_start.min(text_len) as f64)
                                / (text_len as f64);
                            let end_frac =
                                (text_input.cursor_pos.min(text_len) as f64) / (text_len as f64);
                            let sel_start_x = start_frac * text_width as f64;
                            let sel_end_x = end_frac * text_width as f64;
                            let sel_width = (sel_end_x - sel_start_x).abs();
                            if sel_width > 0.0 {
                                let sel_rect = KRect::from_origin_size(
                                    (sel_start_x.min(sel_end_x), 0.0),
                                    (sel_width, text_height as f64),
                                );
                                let selection_color = Color::from_rgba8(0, 122, 255, 40);
                                render_selection(
                                    scene,
                                    align_transform,
                                    &[sel_rect],
                                    selection_color,
                                );
                            }
                        }

                        // === Render IME composition ===
                        if text_input.is_composing && !text_input.composing_text.is_empty() {
                            let (text_width, _) = editor.layout_size();
                            let text_len = editor.text().len().max(1);
                            let cursor_frac = (text_input.composition_start.min(text_len) as f64)
                                / (text_len as f64);
                            let compose_start_x = cursor_frac * text_width as f64;
                            let mut compose_editor = dyxel_editor::Editor::new(node.font_size);
                            compose_editor.set_text(&text_input.composing_text);
                            let compose_color = if text_input.focused {
                                Color::from_rgb8(0, 100, 200)
                            } else {
                                node.color
                            };
                            compose_editor.set_text_color(compose_color);
                            let compose_transform =
                                align_transform * Affine::translate((compose_start_x, 0.0));
                            compose_editor.draw(scene, compose_transform);
                            let (compose_width, compose_height) = compose_editor.layout_size();
                            let underline_rect = KRect::from_origin_size(
                                (0.0, compose_height as f64 + 2.0),
                                (compose_width as f64, 1.5),
                            );
                            scene.fill(
                                Fill::NonZero,
                                compose_transform,
                                compose_color,
                                None,
                                &underline_rect,
                            );
                        }

                        // === Render cursor ===
                        if text_input.focused && text_input.cursor_visible {
                            let (text_width, text_height) = editor.layout_size();
                            let text_len = editor.text().len().max(1);
                            let cursor_frac =
                                (text_input.cursor_pos.min(text_len) as f64) / (text_len as f64);
                            let cursor_x = cursor_frac * text_width as f64;
                            let final_cursor_x = if text_input.is_composing {
                                let mut ce = dyxel_editor::Editor::new(node.font_size);
                                ce.set_text(&text_input.composing_text);
                                cursor_x + ce.layout_size().0 as f64
                            } else {
                                cursor_x
                            };
                            render_cursor(
                                scene,
                                align_transform,
                                final_cursor_x,
                                0.0,
                                text_height as f64,
                                Color::from_rgb8(0, 122, 255),
                            );
                        }
                    } else {
                        // Fallback: draw text even if input state is missing
                        editor.draw(scene, align_transform);
                    }
                }
            } else {
                // Render rectangle at local position
                let rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));

                if node.border_radius > 0.0 {
                    let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                    scene.fill(Fill::NonZero, local_transform, node.color, None, &rounded);

                    // Draw border if border_width > 0
                    if node.border_width > 0.0 {
                        let r = ((node.border_color >> 16) & 0xFF) as u8;
                        let g = ((node.border_color >> 8) & 0xFF) as u8;
                        let b = (node.border_color & 0xFF) as u8;
                        let a = ((node.border_color >> 24) & 0xFF) as u8;
                        let border_color = vello::peniko::Color::from_rgba8(r, g, b, a);
                        let stroke_width = node.border_width as f64;

                        // Create stroke style
                        let stroke = kurbo::Stroke::new(stroke_width);
                        scene.stroke(&stroke, local_transform, border_color, None, &rounded);
                    }
                } else {
                    scene.fill(Fill::NonZero, local_transform, node.color, None, &rect);

                    // Draw border if border_width > 0
                    if node.border_width > 0.0 {
                        let r = ((node.border_color >> 16) & 0xFF) as u8;
                        let g = ((node.border_color >> 8) & 0xFF) as u8;
                        let b = (node.border_color & 0xFF) as u8;
                        let a = ((node.border_color >> 24) & 0xFF) as u8;
                        let border_color = vello::peniko::Color::from_rgba8(r, g, b, a);
                        let stroke_width = node.border_width as f64;

                        // Create stroke style
                        let stroke = kurbo::Stroke::new(stroke_width);
                        scene.stroke(&stroke, local_transform, border_color, None, &rect);
                    }
                }
            }
        }

        // === Step 5: Recursively render children ===
        // Note: Always render children for now since blur texture compositing is disabled
        // TODO: Re-enable !blur_applied check after fixing blur compositing
        if true {
            let local_pos = global_pos + pos_offset;
            for &child_id in &node.children {
                render_node_recursive_with_transform(
                    child_id,
                    state,
                    editors,
                    scene,
                    local_pos,
                    transform,
                    device,
                    queue,
                    renderer,
                    filter_pipeline,
                    blurred_textures,
                    text_input_states.clone(),
                );
            }
        }

        // === Step 6: Pop Layer (if pushed) ===
        // Note: Always pop layer for now since blur texture compositing is disabled
        // TODO: Re-enable !blur_applied check after fixing blur compositing
        if needs_layer {
            scene.pop_layer();
        }
    }
}

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

        // Try using pre-compiled SPIR-V, fall back to WGSL if it fails
        let blit_shader = if cfg!(target_os = "android") {
            let spv_words: Vec<u32> = BLIT_SHADER_SPV
                .chunks_exact(4)
                .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();

            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blit Shader (SPIR-V)"),
                source: wgpu::ShaderSource::SpirV(std::borrow::Cow::Owned(spv_words)),
            })
        } else {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blit Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
            })
        };

        let blit_bl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
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
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let cache_path = format!("{}/vello_v1.cache", config.data_dir);
        log::info!("[ColdStart] Pipeline cache path: {}", cache_path);

        // Detailed cache loading diagnostics with Stage detection
        #[cfg(not(target_arch = "wasm32"))]
        let (cache_stage, cache_data) = match std::fs::read(&cache_path) {
            Ok(data) if data.len() > 1 => {
                // Check for stage marker (first byte)
                let stage = data[0];
                let actual_data = &data[1..];

                match stage {
                    1 => log::info!(
                        "[ColdStart] Stage 1 cache loaded: {} bytes (area_only)",
                        actual_data.len()
                    ),
                    2 => log::info!(
                        "[ColdStart] Stage 2 cache loaded: {} bytes (full)",
                        actual_data.len()
                    ),
                    _ => log::info!("[ColdStart] Legacy cache loaded: {} bytes", data.len()),
                }

                if stage == 1 || stage == 2 {
                    (Some(stage), Some(actual_data.to_vec()))
                } else {
                    // Legacy cache without marker
                    (None, Some(data))
                }
            }
            Ok(_) => {
                log::info!("[ColdStart] Cache file too small, treating as empty");
                (None, None)
            }
            Err(e) => {
                log::warn!(
                    "[ColdStart] Cache file not loaded: {} (path: {})",
                    e,
                    cache_path
                );
                (None, None)
            }
        };
        #[cfg(target_arch = "wasm32")]
        let cache_data: Option<Vec<u8>> = None;

        let pipeline_cache_supported = device.features().contains(wgpu::Features::PIPELINE_CACHE);
        log::info!(
            "[ColdStart] PIPELINE_CACHE feature supported: {}",
            pipeline_cache_supported
        );

        let pipeline_cache = if pipeline_cache_supported {
            let start = std::time::Instant::now();
            let cache = Some(unsafe {
                device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("Vello Pipeline Cache"),
                    data: cache_data.as_deref(),
                    fallback: true,
                })
            });
            log::info!(
                "[ColdStart] Pipeline cache creation took: {:?}",
                start.elapsed()
            );
            cache
        } else {
            log::warn!("[ColdStart] PIPELINE_CACHE not supported, skipping cache");
            None
        };

        *self.blit_bind_group_layout.lock().unwrap() = Some(blit_bl);
        *self.sampler.lock().unwrap() = Some(sampler);
        *self.blit_shader.lock().unwrap() = Some(blit_shader);
        *self.pipeline_cache.lock().unwrap() = pipeline_cache.clone();
        *self.cache_path.lock().unwrap() = Some(cache_path.clone());
        *self.cache_stage.lock().unwrap() = cache_stage;

        // Prewarm blit pipeline
        self.prewarm_pipelines(device, wgpu::TextureFormat::Rgba8Unorm);

        // Initialize filter pipeline for blur effects
        let device_arc = std::sync::Arc::new(device.clone());
        let queue_arc = std::sync::Arc::new(unsafe { &*_queue.as_ptr::<wgpu::Queue>() }.clone());
        match filter_pipeline::FilterPipeline::new(device_arc, queue_arc) {
            Ok(pipeline) => {
                *self.filter_pipeline.lock().unwrap() = Some(pipeline);
                log::info!("[Blur] Filter pipeline initialized successfully");
            }
            Err(e) => {
                log::warn!("[Blur] Failed to initialize filter pipeline: {}", e);
                // Continue without blur support
            }
        }

        // Store info for deferred renderer initialization (includes cache stage)
        *self.init_device_info.lock().unwrap() = Some((cache_path, pipeline_cache, cache_stage));

        // Initialize memory optimizer
        {
            let memory_optimizer = self.memory_optimizer.lock().unwrap();
            memory_optimizer.initialize();
            log::info!(
                "[Memory] Initialized memory optimizer for tier: {:?}",
                memory_optimizer.tier()
            );
        }

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
            log::info!("VelloBackend: VSync disabled by default (Immediate present mode)");
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

        let blit_layout_lock = self.blit_bind_group_layout.lock().unwrap();
        let blit_shader_lock = self.blit_shader.lock().unwrap();

        let device = &v_ctx.devices[v_surface.dev_id].device;

        let bl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[blit_layout_lock.as_ref().unwrap()],
            push_constant_ranges: &[],
        });

        let blit_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&bl),
            vertex: wgpu::VertexState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: v_surface.config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: self.pipeline_cache.lock().unwrap().as_ref(),
        });

        log::info!("VelloBackend: Blit pipeline created successfully");

        #[cfg(target_os = "macos")]
        {
            log::info!("VelloBackend: Creating MacVelloSurfaceState");
            return Ok(Box::new(mac::MacVelloSurfaceState {
                surface: v_surface,
                blit_pipeline: blit_p,
                offscreen_texture: None,
            }));
        }

        #[cfg(target_os = "android")]
        {
            log::info!("VelloBackend: Creating AndroidVelloSurfaceState");
            return Ok(Box::new(android::AndroidVelloSurfaceState {
                surface: v_surface,
                blit_pipeline: blit_p,
                offscreen_texture: None,
            }));
        }

        #[cfg(target_arch = "wasm32")]
        {
            log::info!("VelloBackend: Creating WebVelloSurfaceState");
            return Ok(Box::new(web::WebVelloSurfaceState {
                surface: v_surface,
                blit_pipeline: blit_p,
                offscreen_texture: None,
            }));
        }

        #[cfg(all(
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_arch = "wasm32")
        ))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn prepare(
        &self,
        _shared_state: &SharedPtr<SharedMutex<SharedState>>,
        _width: u32,
        _height: u32,
    ) {
    }

    fn render(
        &self,
        device: DeviceHandle,
        queue: QueueHandle,
        surface: &mut dyn SurfaceState,
        shared_state: &SharedPtr<SharedMutex<SharedState>>,
    ) -> RenderResult {
        // Convert handles to references
        let device = unsafe { &*device.as_ptr::<wgpu::Device>() };
        let queue = unsafe { &*queue.as_ptr::<wgpu::Queue>() };

        #[cfg(target_os = "macos")]
        {
            let v_surface = surface
                .as_any_mut()
                .downcast_mut::<mac::MacVelloSurfaceState>()
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid surface state (not MacVelloSurfaceState)")
                })?;
            return self.render_internal(
                device,
                queue,
                &mut v_surface.surface,
                &v_surface.blit_pipeline,
                &mut v_surface.offscreen_texture,
                shared_state,
            );
        }

        #[cfg(target_os = "android")]
        {
            let v_surface = surface
                .as_any_mut()
                .downcast_mut::<android::AndroidVelloSurfaceState>()
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid surface state (not AndroidVelloSurfaceState)")
                })?;
            return self.render_internal(
                device,
                queue,
                &mut v_surface.surface,
                &v_surface.blit_pipeline,
                &mut v_surface.offscreen_texture,
                shared_state,
            );
        }

        #[cfg(target_arch = "wasm32")]
        {
            let v_surface = surface
                .as_any_mut()
                .downcast_mut::<web::WebVelloSurfaceState>()
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid surface state (not WebVelloSurfaceState)")
                })?;
            return self.render_internal(
                device,
                queue,
                &mut v_surface.surface,
                &v_surface.blit_pipeline,
                &mut v_surface.offscreen_texture,
                shared_state,
            );
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
            Ok(_) => log::debug!("VelloBackend: sync_gpu completed successfully"),
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

/// Render a cursor at the given position
fn render_cursor(
    builder: &mut Scene,
    transform: Affine,
    x: f64,
    y: f64,
    height: f64,
    color: Color,
) {
    let rect = KRect::new(x, y, x + 2.0, y + height);
    builder.fill(Fill::NonZero, transform, color, None, &rect);
}

/// Render selection highlight for multiple rectangles
fn render_selection(builder: &mut Scene, transform: Affine, rects: &[KRect], color: Color) {
    for rect in rects {
        builder.fill(Fill::NonZero, transform, color, None, rect);
    }
}

/// Render focus border for TextInput
fn render_focus_border(
    builder: &mut Scene,
    transform: Affine,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    border_width: f64,
    color: Color,
    border_radius: f64,
) {
    use vello::kurbo::{RoundedRect, Stroke};

    let rect = RoundedRect::new(
        x + border_width / 2.0,
        y + border_width / 2.0,
        x + width - border_width / 2.0,
        y + height - border_width / 2.0,
        border_radius,
    );

    builder.stroke(
        &Stroke::new(border_width),
        transform,
        color,
        None,
        &rect,
    );
}

impl RenderBackendExt for VelloBackend {
    fn enable_perf_overlay(&self) {
        self.enable_perf_overlay();
    }

    fn disable_perf_overlay(&self) {
        self.disable_perf_overlay();
    }
}

impl VelloBackendExt for VelloBackend {
    fn vello_renderer(&self) -> Option<&dyn Any> {
        // Return the backend itself as Any, caller can downcast to VelloBackend
        // and access renderer through the public renderer field
        Some(self as &dyn Any)
    }

    fn update_text_input_state(&self, node_id: u32, state: dyxel_render_api::TextInputRenderState) {
        self.text_input_states
            .lock()
            .unwrap()
            .insert(node_id, state);
    }

    fn remove_text_input_state(&self, node_id: u32) {
        self.text_input_states.lock().unwrap().remove(&node_id);
    }

    fn clear_text_input_states(&self) {
        self.text_input_states.lock().unwrap().clear();
    }
}

/// Factory for creating VelloBackend instances
pub struct VelloBackendFactory;

impl VelloBackendFactory {
    pub fn new() -> Self {
        Self
    }
}

impl dyxel_render_api::RenderBackendFactory for VelloBackendFactory {
    fn create(&self) -> Box<dyn RenderBackend> {
        Box::new(VelloBackend::new())
    }

    fn name(&self) -> &'static str {
        "vello"
    }
}

impl Default for VelloBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for VelloBackendFactory {
    fn default() -> Self {
        Self::new()
    }
}
