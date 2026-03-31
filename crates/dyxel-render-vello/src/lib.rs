// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::any::Any;
use std::sync::atomic::AtomicBool;
use vello::{Renderer, RendererOptions, Scene, peniko::{Color, Fill}};
use dyxel_render_api::{
    RenderBackend, SurfaceState, LifecycleEvent, RenderContext, 
    SharedPtr, SharedMutex, DeviceHandle, QueueHandle, SurfaceTargetHandle, SurfaceHandle,
    RenderResult, BackendConfig, RenderBackendExt, VelloBackendExt
};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use vello::wgpu;
use kurbo::{Affine, Rect as KRect, RoundedRect, Vec2};
use taffy::style::AvailableSpace;
use dyxel_shared::{SharedState, ViewType};
use dyxel_perf::{PerformanceMonitor, SharedPerfMonitor, PerfConfig, PerformanceDiagnostics};

use dyxel_editor::Editor;
// Two-stage init is implemented inline with cache header markers

#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_arch = "wasm32")]
pub mod web;

pub mod staged_init;
pub mod shader_cache;
pub mod minimal_shaders;
pub mod staged_loader;
pub mod two_stage_init;

/// Vello render backend implementation
/// 
/// This is the concrete implementation of RenderBackend using Vello + wgpu
// Type aliases for shared data used in async context
type AsyncShared<T> = std::sync::Arc<std::sync::Mutex<T>>;

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
}

const BLIT_SHADER_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blit.spv"));

impl VelloBackend {
    pub fn new() -> Self {
        Self::with_perf_config(PerfConfig::default())
    }
    
    pub fn with_perf_config(perf_config: PerfConfig) -> Self {
        // Initialize memory optimizer with tiered configuration
        let memory_optimizer = dyxel_perf::MemoryOptimizer::new();
        log::info!("[Memory] VelloBackend: Device tier detected: {:?}", memory_optimizer.tier());
        
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
            perf_monitor: std::sync::Arc::new(std::sync::Mutex::new(PerformanceMonitor::new(perf_config))),
            diagnostics: SharedMutex::new(Some(PerformanceDiagnostics::new(120))),
            overlay_editor: SharedMutex::new(None),
            last_overlay_text: SharedMutex::new(String::new()),
            memory_optimizer: SharedMutex::new(memory_optimizer),
            is_loading: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            loading_handle: SharedMutex::new(None),
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
        
        log::info!("[ColdStart] Cache stage: {:?}, needs_full_load: {}, is_first_launch: {}", 
            cache_stage, needs_full_load, is_first_launch);
        
        // Set loading flag
        self.is_loading.store(true, std::sync::atomic::Ordering::SeqCst);
        
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
                dyxel_perf::DeviceMemoryTier::HighEnd => std::thread::available_parallelism()
                    .ok()
                    .map(|n| n.get()),
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
                    log::info!("[ColdStart] Renderer::new() completed in {:?}", start.elapsed());
                    
                    // Perform minimal warmup
                    let warmup_start = std::time::Instant::now();
                    let dummy_texture = device_clone.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Async Warmup Texture"),
                        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::STORAGE_BINDING,
                        view_formats: &[],
                    });
                    let dummy_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());
                    let scene = Scene::new();
                    let params = vello::RenderParams {
                        base_color: Color::TRANSPARENT,
                        width: 1,
                        height: 1,
                        antialiasing_method: vello::AaConfig::Area,
                    };
                    let _ = renderer.render_to_texture(&device_clone, &queue_clone, &scene, &dummy_view, &params);
                    log::info!("[ColdStart] Warmup completed in {:?}", warmup_start.elapsed());
                    
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
                                    cache_saved_for_thread.store(true, std::sync::atomic::Ordering::SeqCst);
                                    *cache_stage_clone.lock().unwrap() = Some(1);
                                    log::info!("[ColdStart] Stage 1 cache saved ({} bytes)", cache_with_header.len());
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
                            num_init_threads: num_threads.and_then(|n| std::num::NonZeroUsize::new(n)),
                            use_cpu: false,
                        };
                        
                        // Try to create full renderer (will reuse Stage 1 cache + compile remaining)
                        match Renderer::new(&device_clone, full_options) {
                            Ok(full_renderer) => {
                                log::info!("[ColdStart] Stage 2 complete in {:?}", stage2_start.elapsed());
                                
                                // Replace the Stage 1 renderer with full renderer
                                *renderer_clone.lock().unwrap() = Some(full_renderer);
                                
                                // Save Stage 2 cache
                                
                                let cache_lock = pipeline_cache_clone.lock().unwrap();
                                let path_lock = cache_path_clone.lock().unwrap();
                                if let (Some(cache), Some(path)) = (&*cache_lock, &*path_lock) {
                                    if let Some(data) = cache.get_data() {
                                        let mut cache_with_header = Vec::with_capacity(data.len() + 1);
                                        cache_with_header.push(2u8); // Stage 2 marker (full)
                                        cache_with_header.extend_from_slice(&data);
                                        
                                        if std::fs::write(path, &cache_with_header).is_ok() {
                                            log::info!("[ColdStart] Stage 2 cache saved ({} bytes)", cache_with_header.len());
                                            // Update cache_stage to Stage 2
                                            *cache_stage_clone.lock().unwrap() = Some(2);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                log::warn!("[ColdStart] Stage 2 failed: {}, keeping Stage 1 renderer", e);
                            }
                        }
                    }
                    
                    // Record startup performance (Stage 1 time)
                    perf_monitor_clone.lock().unwrap().record_startup_time(start.elapsed());
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
                        log::info!("[ColdStart] Pipeline cache saved successfully ({} bytes)", data.len());
                        self.cache_saved.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                } else {
                    log::warn!("[ColdStart] Cache get_data() returned None");
                }
            }
            #[cfg(target_arch = "wasm32")]
            let _ = (cache, path);
        } else {
            log::warn!("[ColdStart] Cannot save cache: cache={}, path={}", 
                cache_lock.is_some(), path_lock.is_some());
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
                push_constant_ranges: &[]
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Blit Pipeline Prewarm"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL
                    })],
                    compilation_options: Default::default()
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: self.pipeline_cache.lock().unwrap().as_ref()
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
        
        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        
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
        if w == 0 || h == 0 { return Ok(()); }

        // Get or create editors for text nodes and compute layout
        let rid = {
            let mut g = shared_state.lock().unwrap();
            let mut editors = self.editors.lock().unwrap();

            // First pass: create/update editors for text nodes
            for (&id, node) in &g.nodes {
                if node.view_type == ViewType::Text {
                    let editor = editors.entry(id).or_insert_with(|| {
                        let mut ed = Editor::new(node.font_size);
                        ed.set_text(&node.text);
                        // node.color is already peniko::Color
                        ed.set_text_color(node.color);
                        ed
                    });
                    
                    // Update editor if text/font changed
                    if editor.text() != node.text {
                        editor.set_text(&node.text);
                    }
                }
            }

            // Remove editors for deleted nodes
            let node_ids: std::collections::HashSet<u32> = g.nodes.keys().copied().collect();
            editors.retain(|id, _| node_ids.contains(id));

            // Build map from taffy_node to editor id for measurement
            let taffy_to_id: std::collections::HashMap<taffy::NodeId, u32> = g.nodes
                .iter()
                .filter(|(_, n)| n.view_type == ViewType::Text)
                .map(|(id, n)| (n.taffy_node, *id))
                .collect();

            // Compute layout with text measurement
            // First pass: measure with unconstrained width to get natural size
            for (&id, node) in &g.nodes {
                if node.view_type == ViewType::Text {
                    if let Some(editor) = editors.get_mut(&id) {
                        // Measure natural size (no wrapping)
                        editor.set_width(None);
                    }
                }
            }
            

            
            let rid = g.root_id.map(|id| {
                if let Some(rn) = g.nodes.get(&id).map(|n| n.taffy_node) {

                    let _ = g.taffy.compute_layout_with_measure(rn, taffy::prelude::Size {
                        width: AvailableSpace::Definite(w as f32),
                        height: AvailableSpace::Definite(h as f32)
                    }, |_known_dimensions, _available_space, node_id, _node_context, _style| {
                        // Look up editor by taffy_node
                        if let Some(&editor_id) = taffy_to_id.get(&node_id) {
                            if let Some(editor) = editors.get_mut(&editor_id) {
                                // For text nodes: always use natural width (no wrapping)
                                // This prevents unwanted wrapping from parent flex constraints
                                // In the future, we could respect explicit width settings here
                                editor.set_width(None);
                                let (lw, lh) = editor.layout_size();
                                return taffy::geometry::Size { width: lw, height: lh };
                            }
                        }
                        // Not a text node, return default
                        taffy::geometry::Size { 
                            width: _known_dimensions.width.unwrap_or(0.0), 
                            height: _known_dimensions.height.unwrap_or(0.0) 
                        }
                    });
                    
                    // Register all nodes as layout-dirty after computation
                    // This ensures Logic Thread will sync layout to WASM memory
                    {
                        let node_ids: Vec<u32> = g.nodes.keys().copied().collect();
                        dyxel_shared::layout_sync::register_layout_dirty_nodes(&node_ids);

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
            let root_transform = platform_correction(h as f64);
            
            render_node_recursive_with_transform(id, &g, &mut editors, &mut scene, Vec2::ZERO, root_transform);
            stage_timer.mark("scene_build");
        }

        // Get performance stats and draw overlay directly to scene if enabled
        let stats = self.perf_monitor.lock().unwrap().get_stats();
        if should_show_overlay {
            let overlay_text = format!(
                "FPS: {:.1}\nFrame: {:.2}ms\nMem: {:.1}MB\nCPU: {:.1}%",
                stats.fps,
                stats.frame_time_ms,
                stats.memory_used_mb,
                stats.cpu_usage
            );
            
            // Calculate overlay position (top-left corner with padding)
            let (overlay_x, overlay_y, _) = self.perf_monitor.lock().unwrap().get_overlay_config();
            let padding = 10.0;
            let pos_x = padding + overlay_x as f64;
            let pos_y = padding + overlay_y as f64;
            
            // Draw semi-transparent background directly to main scene
            let bg_rect = KRect::new(
                pos_x - 5.0,
                pos_y - 5.0,
                pos_x + 140.0,
                pos_y + 70.0,
            );
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
        if offscreen_texture.as_ref().map_or(true, |(t, _, _)| t.width() != w || t.height() != h) {
            let texture = device.create_texture(&wgpu::TextureDescriptor { 
                label: Some("Vello Offscreen Texture"), 
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, 
                mip_level_count: 1, 
                sample_count: 1, 
                dimension: wgpu::TextureDimension::D2, 
                format: wgpu::TextureFormat::Rgba8Unorm, 
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING, 
                view_formats: &[] 
            });
            let view = texture.create_view(&Default::default());
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor { 
                label: Some("Vello Blit Bind Group"), 
                layout: self.blit_bind_group_layout.lock().unwrap().as_ref().unwrap(), 
                entries: &[
                    wgpu::BindGroupEntry { 
                        binding: 0, 
                        resource: wgpu::BindingResource::TextureView(&view) 
                    }, 
                    wgpu::BindGroupEntry { 
                        binding: 1, 
                        resource: wgpu::BindingResource::Sampler(self.sampler.lock().unwrap().as_ref().unwrap()) 
                    }
                ] 
            });
            *offscreen_texture = Some((texture, view, bg));
        }
        
        let (_, off_view, blit_bg) = offscreen_texture.as_ref().unwrap();
        
        // Tier-based AA configuration: reduce quality for LowEnd to save memory
        let multiplier = self.memory_optimizer.lock().unwrap().vello_buffer_multiplier();
        let aa_config = if multiplier < 0.5 {
            vello::AaConfig::Area // LowEnd: use simpler AA
        } else {
            vello::AaConfig::Area // Default to Area for consistent performance
        };
        
        // Single render: main scene + overlay (if enabled) to offscreen texture
        renderer.render_to_texture(
            device,
            queue,
            &scene,
            off_view,
            &vello::RenderParams {
                base_color: Color::TRANSPARENT,
                width: w,
                height: h,
                antialiasing_method: aa_config
            }
        ).map_err(|e| anyhow::anyhow!("Vello render error: {:?}", e))?;
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
                                store: wgpu::StoreOp::Store 
                            }, 
                            depth_slice: None 
                        })], 
                        depth_stencil_attachment: None, 
                        timestamp_writes: None, 
                        occlusion_query_set: None 
                    }); 
                    rp.set_pipeline(blit_pipeline); 
                    rp.set_bind_group(0, blit_bg, &[]); 
                    rp.draw(0..3, 0..1); 
                }
                queue.submit(Some(enc.finish())); 
                stage_timer.mark("blit_submit");
                st.present();
                stage_timer.mark("present_return");
                
                // After first successful render, save the pipeline cache
                // This ensures cache is complete with all compiled shaders
                static FIRST_RENDER_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if !FIRST_RENDER_DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    log::info!("[ColdStart] First render completed, saving pipeline cache");
                    self.save_cache();
                }
            }
            Err(e) => {
                log::error!("VelloBackend: get_current_texture failed: {:?}", e);
                return Err(anyhow::anyhow!("Surface texture acquisition failed: {:?}", e));
            }
        }
        
        // Log detailed frame timing every 60 frames for diagnostics
        let stats = self.perf_monitor.lock().unwrap().get_stats();
        if stats.total_frames % 60 == 0 {
            let report = stage_timer.report();
            
            // Calculate stage durations
            let state_lock_time = report.get("init_done_to_perf_start") + report.get("perf_start_to_state_lock");
            let scene_build_time = report.get("state_lock_to_scene_build");
            let gpu_time = report.get("scene_build_to_gpu_render");
            let blit_time = report.get("gpu_render_to_blit_submit");
            let present_time = report.get("blit_submit_to_present_return");
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
            log::info!(
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
            if stats.total_frames % 300 == 0 {
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

fn render_node_recursive_with_transform(
    id: u32, 
    state: &SharedState, 
    editors: &mut std::collections::HashMap<u32, Editor>,
    scene: &mut Scene, 
    parent_pos: Vec2, 
    transform: Affine,
) {
    if let Some(node) = state.nodes.get(&id) {
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let taffy_x = layout.location.x as f64;
        let taffy_y = layout.location.y as f64;
        let node_width = layout.size.width as f64;
        let node_height = layout.size.height as f64;
        let global_pos = parent_pos + Vec2::new(taffy_x, taffy_y);
        
        // Build local transform for this node
        let local_transform = transform * Affine::translate((global_pos.x, global_pos.y));
        
        if node.view_type == ViewType::Text {
            // Render text using Editor
            if let Some(editor) = editors.get_mut(&id) {
                editor.set_width(None);
                editor.draw(scene, local_transform);
            }
        } else {
            // Render rectangle at local position
            let rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
            
            if node.border_radius > 0.0 {
                let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                scene.fill(Fill::NonZero, local_transform, node.color, None, &rounded);
            } else {
                scene.fill(Fill::NonZero, local_transform, node.color, None, &rect);
            }
        }
        
        // Recursively render children
        for &child_id in &node.children {
            render_node_recursive_with_transform(child_id, state, editors, scene, global_pos, transform);
        }
    }
}

impl RenderBackend for VelloBackend {
    fn init(&self, device: DeviceHandle, _queue: QueueHandle, config: BackendConfig) -> RenderResult {
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
                source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into())
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
                        multisampled: false
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None
                }
            ]
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
                    1 => log::info!("[ColdStart] Stage 1 cache loaded: {} bytes (area_only)", actual_data.len()),
                    2 => log::info!("[ColdStart] Stage 2 cache loaded: {} bytes (full)", actual_data.len()),
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
                log::warn!("[ColdStart] Cache file not loaded: {} (path: {})", e, cache_path);
                (None, None)
            }
        };
        #[cfg(target_arch = "wasm32")]
        let cache_data: Option<Vec<u8>> = None;
        
        let pipeline_cache_supported = device.features().contains(wgpu::Features::PIPELINE_CACHE);
        log::info!("[ColdStart] PIPELINE_CACHE feature supported: {}", pipeline_cache_supported);
        
        let pipeline_cache = if pipeline_cache_supported {
            let start = std::time::Instant::now();
            let cache = Some(unsafe {
                device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("Vello Pipeline Cache"),
                    data: cache_data.as_deref(),
                    fallback: true,
                })
            });
            log::info!("[ColdStart] Pipeline cache creation took: {:?}", start.elapsed());
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

        // Store info for deferred renderer initialization (includes cache stage)
        *self.init_device_info.lock().unwrap() = Some((cache_path, pipeline_cache, cache_stage));
        
        // Initialize memory optimizer
        {
            let memory_optimizer = self.memory_optimizer.lock().unwrap();
            memory_optimizer.initialize();
            log::info!("[Memory] Initialized memory optimizer for tier: {:?}", memory_optimizer.tier());
        }
        
        log::info!("[Perf] VelloBackend::init: Total time {:?} (Renderer deferred)", init_start.elapsed());
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
        log::info!("VelloBackend: create_surface_state START - size: {}x{}, has_precreated_surface: {}", 
            width, height, surface.is_some());
        
        // Downcast RenderContext to vello::util::RenderContext
        let v_ctx = context.downcast_mut::<vello::util::RenderContext>()
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
            log::info!("VelloBackend: Using pre-created surface (present_mode: {:?})", present_mode);
            let wgpu_surface = s.into_inner::<wgpu::Surface<'static>>()
                .ok_or_else(|| anyhow::anyhow!("SurfaceHandle is not a wgpu::Surface"))?;
            pollster::block_on(v_ctx.create_render_surface(wgpu_surface, width, height, present_mode))
                .map_err(|e| anyhow::anyhow!("Failed to create render surface: {:?}", e))?
        } else if let Some(t) = target {
            log::info!("VelloBackend: Creating surface from target (present_mode: {:?})", present_mode);
            let wgpu_target = t.into_inner::<wgpu::SurfaceTarget<'static>>()
                .ok_or_else(|| anyhow::anyhow!("SurfaceTargetHandle is not a wgpu::SurfaceTarget"))?;
            pollster::block_on(v_ctx.create_surface(wgpu_target, width, height, present_mode))
                .map_err(|e| anyhow::anyhow!("Failed to create surface: {:?}", e))?
        } else {
            return Err(anyhow::anyhow!("Either target or surface must be provided"));
        };
        
        log::info!("VelloBackend: Surface created, format: {:?}, dev_id: {}", v_surface.config.format, v_surface.dev_id);
        
        let blit_layout_lock = self.blit_bind_group_layout.lock().unwrap();
        let blit_shader_lock = self.blit_shader.lock().unwrap();
        
        let device = &v_ctx.devices[v_surface.dev_id].device;

        let bl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[blit_layout_lock.as_ref().unwrap()],
            push_constant_ranges: &[]
        });

        let blit_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&bl),
            vertex: wgpu::VertexState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: v_surface.config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL
                })],
                compilation_options: Default::default()
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: self.pipeline_cache.lock().unwrap().as_ref()
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

        #[cfg(all(not(target_os = "macos"), not(target_os = "android"), not(target_arch = "wasm32")))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn prepare(&self, _shared_state: &SharedPtr<SharedMutex<SharedState>>, _width: u32, _height: u32) {}

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
            let v_surface = surface.as_any_mut().downcast_mut::<mac::MacVelloSurfaceState>()
                .ok_or_else(|| anyhow::anyhow!("Invalid surface state (not MacVelloSurfaceState)"))?;
            return self.render_internal(device, queue, &mut v_surface.surface, &v_surface.blit_pipeline, &mut v_surface.offscreen_texture, shared_state);
        }

        #[cfg(target_os = "android")]
        {
            let v_surface = surface.as_any_mut().downcast_mut::<android::AndroidVelloSurfaceState>()
                .ok_or_else(|| anyhow::anyhow!("Invalid surface state (not AndroidVelloSurfaceState)"))?;
            return self.render_internal(device, queue, &mut v_surface.surface, &v_surface.blit_pipeline, &mut v_surface.offscreen_texture, shared_state);
        }

        #[cfg(target_arch = "wasm32")]
        {
            let v_surface = surface.as_any_mut().downcast_mut::<web::WebVelloSurfaceState>()
                .ok_or_else(|| anyhow::anyhow!("Invalid surface state (not WebVelloSurfaceState)"))?;
            return self.render_internal(device, queue, &mut v_surface.surface, &v_surface.blit_pipeline, &mut v_surface.offscreen_texture, shared_state);
        }

        #[cfg(all(not(target_os = "macos"), not(target_os = "android"), not(target_arch = "wasm32")))]
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
