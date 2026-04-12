// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::any::Any;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use vello::{Renderer, RendererOptions, Scene, peniko::{Color, Fill}};
use dyxel_render_api::{
    RenderBackend, SurfaceState, LifecycleEvent, RenderContext, 
    SharedPtr, SharedMutex, DeviceHandle, QueueHandle, SurfaceTargetHandle, SurfaceHandle,
    RenderResult, BackendConfig, RenderBackendExt, VelloBackendExt
};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use vello::wgpu;
use kurbo::{Affine, Rect as KRect, Vec2};
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
pub mod scene_adapter;
pub mod filter_pipeline;
pub mod texture_pool;

/// Vello render backend implementation
/// 
/// This is the concrete implementation of RenderBackend using Vello + wgpu
// Type aliases for shared data used in async context
type AsyncShared<T> = std::sync::Arc<std::sync::Mutex<T>>;

/// Entry for a blurred texture to be composited
#[derive(Debug)]
struct BlurredTextureEntry {
    /// The blurred texture (contains blurred background for frosted glass)
    texture: wgpu::Texture,
    /// Width of the texture
    width: u32,
    /// Height of the texture
    height: u32,
    /// Position to draw at (with padding offset already applied)
    transform: Affine,
    /// Opacity of the blurred content
    opacity: f32,
    /// View color to overlay (for frosted glass effect)
    overlay_color: vello::peniko::Color,
    /// Border radius
    border_radius: f64,
    /// Source rectangle in scene texture (for two-pass rendering)
    source_rect: (f32, f32, f32, f32), // (x, y, width, height) in scene coordinates
    /// Deferred children to render on top of blurred background
    deferred_children: Vec<u32>,
    /// View ID for deferred rendering
    view_id: u32,
    /// Blur radius
    blur_radius: f32,
    /// Blur style: 0=Light, 1=Dark, 2=ExtraLight, 3=Prominent
    blur_style: u8,
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
    // Pipeline for rendering children texture with alpha blending
    children_blit_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
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
    // Blur composite pipeline for drawing blurred textures
    blur_composite_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    blur_composite_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    blur_composite_uniforms: SharedMutex<Option<wgpu::Buffer>>,
    blur_composite_overlay_uniforms: SharedMutex<Option<wgpu::Buffer>>,
    // Blurred textures to composite (cleared each frame)
    blurred_textures: SharedMutex<Vec<BlurredTextureEntry>>,
    // Texture pool for efficient blur texture reuse
    texture_pool: SharedMutex<Option<texture_pool::SharedTexturePool>>,
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
            children_blit_pipeline: SharedMutex::new(None),
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
            filter_pipeline: SharedMutex::new(None),
            blur_composite_pipeline: SharedMutex::new(None),
            blur_composite_bind_group_layout: SharedMutex::new(None),
            blur_composite_uniforms: SharedMutex::new(None),
            blur_composite_overlay_uniforms: SharedMutex::new(None),
            blurred_textures: SharedMutex::new(Vec::new()),
            texture_pool: SharedMutex::new(None),
        }
    }
    
    /// Save texture to PNG file for debugging
    #[cfg(not(target_arch = "wasm32"))]
    fn save_texture_to_png(&self, device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, path: &str) {
        let size = texture.size();
        let format = texture.format();

        // wgpu requires bytes_per_row to be a multiple of 256
        let bytes_per_pixel = match format {
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm => 4,
            _ => 4,
        };
        let bytes_per_row_unaligned = size.width * bytes_per_pixel;
        let bytes_per_row = ((bytes_per_row_unaligned + 255) / 256) * 256;
        let buffer_size = (bytes_per_row * size.height) as u64;

        log::debug!("[DebugSave] Saving {}: {}x{} format={:?} bytes_per_row={}",
            path, size.width, size.height, format, bytes_per_row);

        // Create buffer to read back
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Readback Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // Copy texture to buffer
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        queue.submit(Some(encoder.finish()));

        // Map and save
        let buffer_slice = readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        // Poll device until mapping completes
        while rx.try_recv().is_err() {
            let _ = device.poll(wgpu::PollType::Poll);
        }

        {
            let data = buffer_slice.get_mapped_range();
            let rgba_data: &[u8] = &data;

            // Check first few pixels to see if texture is empty
            let first_pixels: Vec<[u8; 4]> = rgba_data.chunks(4).take(4)
                .filter_map(|c| c.try_into().ok()).collect();
            log::debug!("[DebugSave] First 4 pixels: {:?}", first_pixels);

            // Debug: Sample pixels at multiple locations
            for (sx, sy) in [(763u32, 598u32), (1185u32, 598u32), (1606u32, 598u32)] {
                let sample_offset = (sy * bytes_per_row + sx * bytes_per_pixel) as usize;
                if sample_offset + 3 < rgba_data.len() {
                    let r = rgba_data[sample_offset];
                    let g = rgba_data[sample_offset + 1];
                    let b = rgba_data[sample_offset + 2];
                    let a = rgba_data[sample_offset + 3];
                    log::debug!("[DebugSave] Scene pixel at ({},{}): RGBA=({},{},{},{})", sx, sy, r, g, b, a);
                }
            }

            // Copy row by row to handle alignment
            let mut img_data = Vec::with_capacity((size.width * size.height * 3) as usize);
            for row in 0..size.height {
                let row_start = (row * bytes_per_row) as usize;
                for col in 0..size.width {
                    let pixel_offset = row_start + (col * bytes_per_pixel) as usize;
                    if pixel_offset + 2 < rgba_data.len() {
                        // Handle BGRA vs RGBA
                        if format == wgpu::TextureFormat::Bgra8Unorm {
                            img_data.push(rgba_data[pixel_offset + 2]); // R (from B)
                            img_data.push(rgba_data[pixel_offset + 1]); // G
                            img_data.push(rgba_data[pixel_offset]);     // B (from R)
                        } else {
                            img_data.push(rgba_data[pixel_offset]);     // R
                            img_data.push(rgba_data[pixel_offset + 1]); // G
                            img_data.push(rgba_data[pixel_offset + 2]); // B
                        }
                    }
                }
            }

            let img = image::RgbImage::from_raw(size.width, size.height, img_data);
            if let Some(img) = img {
                if let Err(e) = img.save(path) {
                    log::warn!("Failed to save debug image {}: {}", path, e);
                } else {
                    log::info!("Saved debug image: {}", path);
                }
            } else {
                log::warn!("Failed to create image from raw data");
            }
        }
        readback_buffer.unmap();
    }

    /// Check if debug frame saving is enabled
    #[cfg(not(target_arch = "wasm32"))]
    fn debug_frames_enabled(&self) -> bool {
        false // Disabled: was std::env::var("DYXEL_DEBUG_FRAMES").is_ok()
    }

    /// Get debug output directory
    #[cfg(not(target_arch = "wasm32"))]
    fn debug_output_dir(&self) -> std::path::PathBuf {
        let dir = std::env::var("DYXEL_DEBUG_DIR").unwrap_or_else(|_| "debug_frames".to_string());
        let path = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&path).ok();
        path
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
            log::info!("[ColdStart] Cache already saved, skipping");
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

    /// Initialize blur composite pipeline for drawing blurred textures
    fn init_blur_composite_pipeline(&self, device: &wgpu::Device) {
        // Default to Rgba8Unorm, will be recreated with correct format if needed
        self.create_blur_composite_pipeline(device, wgpu::TextureFormat::Rgba8Unorm);
    }

    /// Create blur composite pipeline with specific format
    fn create_blur_composite_pipeline(&self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        // Create bind group layout with uniform buffer for transform and overlay
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blur Composite Bind Group Layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create uniform buffer (3 rows of vec4 = 48 bytes)
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blur Composite Uniform Buffer"),
            size: 48, // 3 * 16 bytes (aligned vec4s)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create overlay uniform buffer (color + radius + size = 32 bytes)
        let overlay_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blur Overlay Uniform Buffer"),
            size: 32, // 2 * 16 bytes (aligned vec4s)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Load shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blur Composite Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blur_composite.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blur Composite Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blur Composite Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: format,
                    // Premultiplied alpha blending: shader outputs premultiplied colors
                    // src_factor=One because RGB is already multiplied by alpha
                    // This correctly composites frosted glass over the main scene
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        *self.blur_composite_pipeline.lock().unwrap() = Some(pipeline);
        *self.blur_composite_bind_group_layout.lock().unwrap() = Some(bind_group_layout);
        *self.blur_composite_uniforms.lock().unwrap() = Some(uniform_buffer);
        *self.blur_composite_overlay_uniforms.lock().unwrap() = Some(overlay_uniform_buffer);

        log::debug!("[Blur] Composite pipeline initialized");
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
        #[cfg(not(target_os = "android"))]
        let frame_start = std::time::Instant::now();
        let mut stage_timer = dyxel_perf::FrameTimer::new();
        
        // Async initialization: start background compilation without blocking
        self.ensure_renderer_initialized_async(device, queue);
        stage_timer.mark("init_check");
        
        // Check if renderer is ready
        let mut renderer_lock = self.renderer.lock().unwrap();
        let renderer = match renderer_lock.as_mut() {
            Some(r) => {
                log::info!("[DIAG] Renderer ready");
                r
            }
            None => {
                // Renderer not ready yet - clear surface and return
                log::info!("[DIAG] Renderer not ready, clearing surface");
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

        // Collect returned textures from previous frame
        if let Some(ref pool) = *self.texture_pool.lock().unwrap() {
            pool.collect_returns();
        }

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
                        log::info!("[Editor] Creating editor for node {} with text_color: {:?}", id, node.text_color);
                        ed.set_text_color(node.text_color);
                        ed
                    });
                    
                    // Update editor if text changed
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
                        if (new_width - old_width).abs() > 0.5 || (new_height - old_height).abs() > 0.5 {
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
            let root_transform = platform_correction(h as f64);

            // Get filter pipeline for blur effects
            let filter_pipeline = self.filter_pipeline.lock().unwrap();
            let mut blurred_textures = self.blurred_textures.lock().unwrap();

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
            );
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
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, 
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
        log::debug!("[Blur] Rendering scene to texture {}x{}", w, h);
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

        // IMPORTANT: Submit and wait for scene rendering to complete before copying
        // This ensures the scene texture has valid content for Pass 2 blur sampling
        queue.submit(None);
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).ok();

        // Debug: Save scene texture after Pass 1
        #[cfg(not(target_arch = "wasm32"))]
        if self.debug_frames_enabled() {
            let frame_num = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let debug_dir = self.debug_output_dir();
            if let Some((scene_tex, _, _)) = offscreen_texture.as_ref() {
                let path = debug_dir.join(format!("frame_{:06}_pass1_scene.png", frame_num % 1000));
                self.save_texture_to_png(device, queue, scene_tex, path.to_str().unwrap());

                // Debug: Sample pixels at blur card locations (expected to show purple background)
                log::debug!("[Debug] Sampling scene texture at blur card locations (expected purple bg)");
            }
        }

        // === PASS 2: Process blur textures from scene ===
        // OPTIMIZED: Batch all texture copies into a single command buffer
        {
            let blurred_textures = self.blurred_textures.lock().unwrap();
            let filter_pipeline = self.filter_pipeline.lock().unwrap();

            log::debug!("[Blur Pass 2] Starting with {} blur entries", blurred_textures.len());

            if !blurred_textures.is_empty() {
                if let Some(pipeline) = filter_pipeline.as_ref() {
                    let scene_texture = offscreen_texture.as_ref().map(|(t, _, _)| t)
                        .expect("Scene texture should exist");

                    // Create a single encoder for all copy operations
                    let mut copy_enc = device.create_command_encoder(
                        &wgpu::CommandEncoderDescriptor {
                            label: Some("Blur Pass 2 - Batch Copy Encoder"),
                        });

                    // Collect entries that need blur processing
                    let mut blur_entries: Vec<_> = Vec::new();

                    for entry in blurred_textures.iter() {
                        // Copy the region from scene texture to blur texture
                        // The source rectangle is in screen coordinates
                        let (src_x, src_y, src_w, src_h) = entry.source_rect;

                        log::debug!("[Blur] Copying region: src=({:.0},{:.0}) size={:.0}x{:.0} to blur texture {}x{}",
                            src_x, src_y, src_w, src_h, entry.width, entry.height);
                        log::debug!("[Blur] Checking scene content at src=({:.0},{:.0}) - parent bg is at y=578 (id=37)", src_x, src_y);

                        // Collect entries that need blur processing
                        if entry.blur_radius > 0.0 {
                            log::debug!("[Blur Pass 2] Collecting view_id={} for blur processing, radius={}", entry.view_id, entry.blur_radius);
                            blur_entries.push((
                                entry.view_id,
                                &entry.texture,
                                entry.blur_radius,
                            ));
                        }

                        // Clear blur texture to transparent before copying background
                        copy_enc.clear_texture(
                            &entry.texture,
                            &wgpu::ImageSubresourceRange {
                                aspect: wgpu::TextureAspect::All,
                                base_mip_level: 0,
                                mip_level_count: None,
                                base_array_layer: 0,
                                array_layer_count: None,
                            },
                        );

                        // Calculate padding and coordinates
                        let padding = ((entry.width as f32 - src_w) / 2.0) as u32;

                        #[cfg(target_os = "android")]
                        let src_origin_y = (h as f32 - src_y - src_h).max(0.0) as u32;
                        #[cfg(not(target_os = "android"))]
                        let src_origin_y = src_y.max(0.0) as u32;

                        let src_origin_x = src_x.max(0.0) as u32;
                        let copy_width = src_w as u32;
                        let copy_height = src_h as u32;

                        // Queue copy command
                        copy_enc.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: scene_texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: src_origin_x,
                                    y: src_origin_y,
                                    z: 0,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::TexelCopyTextureInfo {
                                texture: &entry.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: padding,
                                    y: padding,
                                    z: 0,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::Extent3d {
                                width: copy_width,
                                height: copy_height,
                                depth_or_array_layers: 1,
                            },
                        );
                    }

                    // Submit all copy commands at once
                    queue.submit(std::iter::once(copy_enc.finish()));

                    // IMPORTANT: Wait for all copies to complete before applying blur
                    // This ensures the blur textures have valid scene content
                    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).ok();

                    stage_timer.mark("blur_copy_submit");

                    // NOTE: We cannot batch multiple blur operations into a single encoder
                    // because they share intermediate textures from the pool.
                    // Each blur must complete before the next one starts to avoid texture conflicts.
                    log::debug!("[Blur Pass 2] Processing {} blur entries", blur_entries.len());
                    for (view_id, texture, blur_radius) in blur_entries {
                        log::debug!("[Blur Pass 2] Applying Kawase frosted glass: view_id={}, radius={}", view_id, blur_radius);

                        let pool_guard = self.texture_pool.lock().unwrap();
                        let result = if let Some(ref pool) = *pool_guard {
                            pipeline.apply_frosted_glass_kawase(
                                texture,
                                texture,
                                blur_radius,
                                Some(pool),
                            )
                        } else {
                            pipeline.apply_frosted_glass_kawase(
                                texture,
                                texture,
                                blur_radius,
                                None,
                            )
                        };

                        if let Err(e) = result {
                            log::warn!("[Blur] Failed to apply Kawase frosted glass for view {}: {:?}", view_id, e);
                        } else {
                            log::debug!("[Blur] Kawase frosted glass applied successfully for view {}", view_id);
                        }
                    }
                    stage_timer.mark("blur_render_submit");

                    }
                }
            }

        stage_timer.mark("pass3_start");

        // === PASS 3: Render deferre d children ===
        // === PASS 3: Render deferred children to separate texture ===
        // Create a texture for children that will be drawn AFTER blur textures
        // This ensures children appear sharp on top of the blurred background
        let mut children_scene = Scene::new();
        let mut has_children = false;

        {
            let blurred_textures = self.blurred_textures.lock().unwrap();

            for entry in blurred_textures.iter() {
                if entry.deferred_children.is_empty() {
                    continue;
                }
                has_children = true;

                let g = shared_state.lock().unwrap();
                let mut editors = self.editors.lock().unwrap();

                // Use source_rect to get the blur node's actual screen position
                // entry.transform includes a -padding offset, so we use source_rect directly
                let global_x = entry.source_rect.0 as f64;
                let global_y = entry.source_rect.1 as f64;

                // Render each deferred child
                for &child_id in &entry.deferred_children {
                    render_deferred_child(
                        child_id,
                        &g,
                        &mut editors,
                        &mut children_scene,
                        Vec2::new(global_x, global_y),
                    );
                }
            }
        }

        // Create or update children texture
        let children_texture = if has_children {
            log::debug!("[Blur] Pass 3: Rendering children texture");
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Children Texture"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Render children scene to texture
            if let Err(e) = renderer.render_to_texture(
                device,
                queue,
                &children_scene,
                &view,
                &vello::RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: w,
                    height: h,
                    antialiasing_method: aa_config,
                }
            ) {
                log::warn!("[Blur] Failed to render children texture: {:?}", e);
                None
            } else {
                // Debug: Save children texture
                #[cfg(not(target_arch = "wasm32"))]
                if self.debug_frames_enabled() {
                    let frame_num = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let debug_dir = self.debug_output_dir();
                    let path = debug_dir.join(format!("frame_{:06}_pass3_children.png", frame_num % 1000));
                    self.save_texture_to_png(device, queue, &texture, path.to_str().unwrap());
                }
                Some((texture, view))
            }
        } else {
            None
        };
        stage_timer.mark("pass3_done");

        // Single present: blit the combined result (main scene + optional overlay) to screen
        match v_surface_surface.surface.get_current_texture() {
            Ok(st) => {
                // Debug: Get frame number before render pass
                #[cfg(not(target_arch = "wasm32"))]
                let debug_frame_num = if self.debug_frames_enabled() {
                    let frame_num = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    Some(frame_num % 1000)
                } else { None };

                // Debug: Create capture texture if needed
                #[cfg(not(target_arch = "wasm32"))]
                let capture_texture = if self.debug_frames_enabled() && debug_frame_num.is_some() {
                    let capture_tex = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Capture Texture"),
                        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: st.texture.format(),
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    });
                    Some(capture_tex)
                } else {
                    None
                };

                // Determine render target
                #[cfg(not(target_arch = "wasm32"))]
                let render_target_view = if let Some(ref capture_tex) = capture_texture {
                    capture_tex.create_view(&Default::default())
                } else {
                    st.texture.create_view(&Default::default())
                };
                #[cfg(target_arch = "wasm32")]
                let render_target_view = st.texture.create_view(&Default::default());

                let mut enc = device.create_command_encoder(&Default::default());
                // Track whether we rendered any blur textures this frame
                let mut had_blur_textures = false;
                {
                    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Vello Blit Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_target_view,
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

                    // Draw blurred textures using composite pipeline
                    log::debug!("[Blur Pass 4] About to lock blurred_textures for compositing");
                    let blurred_textures = self.blurred_textures.lock().unwrap();
                    log::debug!("[Blur Pass 4] Locked blurred_textures, count = {}", blurred_textures.len());
                    if !blurred_textures.is_empty() {
                        log::debug!("[Blur] COMPOSITING {} blurred textures", blurred_textures.len());

                        // Create pipeline with correct surface format if needed
                        // Must match the render pass format (Bgra8Unorm on macOS)
                        let surface_format = v_surface_surface.config.format;
                        log::debug!("[Blur] Surface config format: {:?}", surface_format);

                        // Create pipeline only if it doesn't exist (avoid expensive recreation every frame)
                        log::debug!("[Blur] Checking if pipeline needs creation...");
                        let needs_pipeline = {
                            let guard = self.blur_composite_pipeline.lock();
                            match guard {
                                Ok(g) => g.is_none(),
                                Err(e) => {
                                    log::error!("[Blur] Pipeline mutex poisoned: {}", e);
                                    e.into_inner().is_none()
                                }
                            }
                            // Lock released here at end of block
                        };
                        log::debug!("[Blur] needs_pipeline = {}", needs_pipeline);
                        if needs_pipeline {
                            log::debug!("[Blur] Creating composite pipeline with surface format {:?}", surface_format);
                            self.create_blur_composite_pipeline(device, surface_format);
                            log::debug!("[Blur] Pipeline creation complete");
                        }

                        // Get the blur composite pipeline (handle poisoned mutex)
                        log::debug!("[Blur] Acquiring pipeline lock...");
                        let blur_pipeline = match self.blur_composite_pipeline.lock() {
                            Ok(g) => g,
                            Err(e) => {
                                log::warn!("[Blur] Pipeline mutex poisoned, recovering");
                                e.into_inner()
                            }
                        };
                        let blur_bg_layout = match self.blur_composite_bind_group_layout.lock() {
                            Ok(g) => g,
                            Err(e) => {
                                log::warn!("[Blur] Layout mutex poisoned, recovering");
                                e.into_inner()
                            }
                        };
                        let uniform_buffer = match self.blur_composite_uniforms.lock() {
                            Ok(g) => g,
                            Err(e) => {
                                log::warn!("[Blur] Uniforms mutex poisoned, recovering");
                                e.into_inner()
                            }
                        };
                        let overlay_uniform_buffer = match self.blur_composite_overlay_uniforms.lock() {
                            Ok(g) => g,
                            Err(e) => {
                                log::warn!("[Blur] Overlay mutex poisoned, recovering");
                                e.into_inner()
                            }
                        };
                        log::debug!("[Blur] Got all locks");

                        let pipeline_ready = blur_pipeline.is_some();
                        let layout_ready = blur_bg_layout.is_some();
                        let uniforms_ready = uniform_buffer.is_some();
                        let overlay_ready = overlay_uniform_buffer.is_some();

                        if !(pipeline_ready && layout_ready && uniforms_ready && overlay_ready) {
                            log::warn!("[Blur] Resources not ready: pipeline={}, layout={}, uniforms={}, overlay={}",
                                pipeline_ready, layout_ready, uniforms_ready, overlay_ready);
                        }

                        if let (Some(pipeline), Some(layout), _, _) =
                            (blur_pipeline.as_ref(), blur_bg_layout.as_ref(), uniform_buffer.as_ref(), overlay_uniform_buffer.as_ref()) {

                            log::debug!("[Blur] All resources ready, starting draw loop for {} textures", blurred_textures.len());

                            // Create sampler
                            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                                mag_filter: wgpu::FilterMode::Linear,
                                min_filter: wgpu::FilterMode::Linear,
                                ..Default::default()
                            });

                            // Draw each blurred texture
                            log::debug!("[Blur Pass 4] Starting compositing loop for {} textures", blurred_textures.len());
                            for entry in blurred_textures.iter() {
                                // Convert transform to matrix
                                // Vello uses [xx, yx, xy, yy, x0, y0] order
                                // We need to convert to clip space (-1 to 1)
                                let affine = entry.transform;
                                let mat = affine.as_coeffs();
                                // mat = [xx, yx, xy, yy, x0, y0]

                                // Screen space to clip space conversion
                                // clip_x = (screen_x / width) * 2 - 1
                                // clip_y = 1 - (screen_y / height) * 2
                                let scale_x = 2.0 / w as f32;
                                let scale_y = -2.0 / h as f32;
                                let offset_x = -1.0;
                                let offset_y = 1.0;

                                // Transform matrix components
                                // We want to transform from (0,0)-(1,1) UV space to clip space
                                // But also apply the affine transform from Vello
                                //
                                // IMPORTANT: The UV space (0,1) needs to be scaled by texture size
                                // to get pixel coordinates in local space, then apply Vello transform,
                                // then convert to clip space.
                                //
                                // Correct formula:
                                //   local_pos = uv * texture_size
                                //   screen_pos = M * local_pos + T
                                //   clip_pos = screen_pos * clip_scale + clip_offset
                                //
                                // So: clip_pos = (M * uv * texture_size + T) * clip_scale + clip_offset
                                //              = (M * texture_size * clip_scale) * uv + (T * clip_scale + clip_offset)

                                let tex_width = entry.width as f32;
                                let tex_height = entry.height as f32;

                                let uniform_data: [f32; 12] = [
                                    // Row 0: m00, m01, pad, pad
                                    // m00 scales UV x (0..1) to screen x, considering texture width and clip scale
                                    mat[0] as f32 * tex_width * scale_x,
                                    mat[2] as f32 * tex_width * scale_x,
                                    0.0, 0.0,
                                    // Row 1: m10, m11, pad, pad
                                    // m11 scales UV y (0..1) to screen y, considering texture height and clip scale
                                    mat[1] as f32 * tex_height * scale_y,
                                    mat[3] as f32 * tex_height * scale_y,
                                    0.0, 0.0,
                                    // Row 2: tx, ty, opacity, pad
                                    mat[4] as f32 * scale_x + offset_x,
                                    mat[5] as f32 * scale_y + offset_y,
                                    entry.opacity,
                                    0.0,
                                ];

                                // Create per-entry uniform buffer to avoid data races
                                let entry_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                                    label: Some(&format!("Blur Uniform Buffer {}", entry.view_id)),
                                    size: 48, // 12 * 4 bytes
                                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                                    mapped_at_creation: false,
                                });
                                queue.write_buffer(&entry_uniforms, 0, bytemuck::cast_slice(&uniform_data));

                                // Write overlay uniform data
                                // Use the node's overlay_color for tint
                                let overlay_color = entry.overlay_color;
                                let overlay_data: [f32; 8] = [
                                    // Row 0: color_r, color_g, color_b, color_a
                                    // AlphaColor stores components as [r, g, b, a] in f32
                                    overlay_color.components[0],
                                    overlay_color.components[1],
                                    overlay_color.components[2],
                                    overlay_color.components[3],
                                    // Row 1: border_radius, view_width, view_height, color_mode
                                    entry.border_radius as f32,
                                    entry.width as f32,
                                    entry.height as f32,
                                    if entry.blur_style == 1 || entry.blur_style == 3 { 1.0f32 } else { 0.0f32 },
                                ];

                                // Create per-entry overlay uniform buffer
                                let entry_overlay_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                                    label: Some(&format!("Blur Overlay Uniform Buffer {}", entry.view_id)),
                                    size: 32, // 8 * 4 bytes
                                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                                    mapped_at_creation: false,
                                });
                                queue.write_buffer(&entry_overlay_uniforms, 0, bytemuck::cast_slice(&overlay_data));

                                log::debug!("[Blur] Uniform data: {:?}", uniform_data);
                                log::debug!("[Blur] Overlay data: {:?}", overlay_data);
                                log::debug!("[Blur] Texture size: {}x{}", entry.width, entry.height);

                                let texture_view = entry.texture.create_view(
                                    &wgpu::TextureViewDescriptor::default());

                                let bind_group = device.create_bind_group(
                                    &wgpu::BindGroupDescriptor {
                                        label: Some(&format!("Blur Composite Bind Group {}", entry.view_id)),
                                        layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: wgpu::BindingResource::TextureView(&texture_view),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::Sampler(&sampler),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: entry_uniforms.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: entry_overlay_uniforms.as_entire_binding(),
                                            },
                                        ],
                                    });

                                    log::debug!("[Blur Pass 4] Compositing view_id={} alpha={}", entry.view_id, entry.opacity);

                                rp.set_pipeline(pipeline);
                                rp.set_bind_group(0, &bind_group, &[]);
                                rp.draw(0..6, 0..1); // 6 vertices for quad (2 triangles)
                                log::debug!("[Blur Pass 4] Drew view_id={}", entry.view_id);
                            }
                        }
                    }
                    // Capture whether we had blur textures before clearing
                    had_blur_textures = !blurred_textures.is_empty();
                    log::debug!("[Blur Pass 4] Composited {} blur textures, had_blur_textures = {}", blurred_textures.len(), had_blur_textures);
                    // Clear blurred textures after drawing
                    drop(blurred_textures);
                    self.blurred_textures.lock().unwrap().clear();

                    // === Draw children texture on top of blur ===
                    // This ensures children appear sharp on top of the blurred background
                    if let Some((_, ref children_view)) = children_texture {
                        // Create bind group for children texture using the same layout as blit
                        let children_bind_group = device.create_bind_group(
                            &wgpu::BindGroupDescriptor {
                                label: Some("Children Blit Bind Group"),
                                layout: self.blit_bind_group_layout.lock().unwrap().as_ref().unwrap(),
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::TextureView(children_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::Sampler(
                                            self.sampler.lock().unwrap().as_ref().unwrap()
                                        ),
                                    },
                                ],
                            });

                        // Draw children texture with alpha blending on top of everything
                        // Use children_blit_pipeline which has ALPHA_BLENDING
                        if let Some(ref children_pipeline) = *self.children_blit_pipeline.lock().unwrap() {
                            rp.set_pipeline(children_pipeline);
                        } else {
                            // Fallback to blit_pipeline if children pipeline not ready
                            rp.set_pipeline(blit_pipeline);
                        }
                        rp.set_bind_group(0, &children_bind_group, &[]);
                        rp.draw(0..3, 0..1);
                    }
                }

                // If using capture texture, blit it to surface before present
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(ref capture_tex) = capture_texture {
                    // Create bind group for capture texture blit
                    let capture_bind_group = device.create_bind_group(
                        &wgpu::BindGroupDescriptor {
                            label: Some("Capture Blit Bind Group"),
                            layout: self.blit_bind_group_layout.lock().unwrap().as_ref().unwrap(),
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(
                                        &capture_tex.create_view(&Default::default())
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::Sampler(
                                        self.sampler.lock().unwrap().as_ref().unwrap()
                                    ),
                                },
                            ],
                        });

                    {
                        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Capture Blit Pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &st.texture.create_view(&Default::default()),
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                    store: wgpu::StoreOp::Store
                                },
                                depth_slice: None
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None
                        });
                        rp.set_pipeline(blit_pipeline);
                        rp.set_bind_group(0, &capture_bind_group, &[]);
                        rp.draw(0..3, 0..1);
                    }
                }

                // Debug: Save composite frame when we have blur textures
                // NOTE: We handle submission inside the block to avoid double-submit
                #[cfg(not(target_arch = "wasm32"))]
                {
                    log::debug!("[Debug] Checking had_blur_textures = {}", had_blur_textures);
                    if had_blur_textures && self.debug_frames_enabled() {
                        // IMPORTANT: Submit encoder first to ensure all drawing is complete
                        queue.submit(Some(enc.finish()));

                        if let Some(capture_tex) = &capture_texture {
                            let debug_dir = self.debug_output_dir();
                            let frame_num = debug_frame_num.unwrap_or(0);
                            let capture_path = debug_dir.join(format!("frame_{:06}_pass0_composite.png", frame_num));
                            log::debug!("[DebugSave] AFTER SUBMIT - Saving composite frame to {:?}", capture_path);
                            self.save_texture_to_png(device, queue, capture_tex, capture_path.to_str().unwrap());
                        }

                        // Create new encoder for present (since we submitted the old one)
                        enc = device.create_command_encoder(&Default::default());
                    } else {
                        // Submit normally
                        queue.submit(Some(enc.finish()));
                    }
                }

                #[cfg(target_arch = "wasm32")]
                {
                    queue.submit(Some(enc.finish()));
                }
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
        
        // Log detailed frame timing for diagnostics
        let stats = self.perf_monitor.lock().unwrap().get_stats();
        {
            let report = stage_timer.report();
            
            // Calculate stage durations
            #[cfg(not(target_os = "android"))]
            let state_lock_time = report.get("init_check_to_perf_start") + report.get("perf_start_to_state_lock");
            #[cfg(not(target_os = "android"))]
            let scene_build_time = report.get("state_lock_to_scene_build");
            #[cfg(not(target_os = "android"))]
            let gpu_time = report.get("scene_build_to_gpu_render");
            #[cfg(not(target_os = "android"))]
            let blur_copy_time = report.get("gpu_render_to_blur_copy_submit");
            #[cfg(not(target_os = "android"))]
            let blur_render_time = report.get("blur_copy_submit_to_blur_render_submit");
            #[cfg(not(target_os = "android"))]
            let pass3_time = report.get("blur_render_submit_to_pass3_done");
            #[cfg(not(target_os = "android"))]
            let blit_time = report.get("pass3_done_to_blit_submit");
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
            {
                // Log FPS every frame
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
            }

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
        // Android: Vello renders Y-up, need flip to match screen Y-down
        Affine::translate((0.0, viewport_height)) * Affine::scale_non_uniform(1.0, -1.0)
    }
    #[cfg(not(target_os = "android"))]
    {
        // macOS/iOS: Vello's render_to_texture already produces Y-down output
        let _ = viewport_height;
        Affine::IDENTITY
    }
}

/// Render node content with blur effect applied (Two-pass frosted glass)
///
/// In the two-pass approach:
/// 1. First pass: Render all content to scene texture (done by caller)
/// 2. Second pass: Sample from scene texture, apply blur, overlay color
///
/// This function prepares the blur entry for the second pass.
fn render_with_blur(
    node: &dyxel_shared::ViewNode,
    id: u32,
    _state: &SharedState,
    _editors: &mut std::collections::HashMap<u32, Editor>,
    _scene: &mut Scene,
    local_transform: Affine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    filter_pipeline: &crate::filter_pipeline::FilterPipeline,
    node_width: f64,
    node_height: f64,
    needs_layer: bool,
    blurred_textures: &mut Vec<BlurredTextureEntry>,
) -> bool {
    // Unused imports - kept for reference but not needed in two-pass approach
    // use vello::peniko::{Fill, Color};
    // use kurbo::{Rect as KRect, RoundedRect};

    // Calculate padded size for blur (need extra space for blur bleed)
    let blur_radius = node.blur_radius as f64;
    let padding = (blur_radius * 2.5).ceil() as u32;
    let texture_width = (node_width as u32 + padding * 2).max(1);
    let texture_height = (node_height as u32 + padding * 2).max(1);

    // Create offscreen texture for the blurred result
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
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    };

    let offscreen_texture = device.create_texture(&texture_desc);

    // NOTE: For true two-pass frosted glass, we don't render anything here.
    // The blur texture will be created AFTER the main scene is rendered,
    // by sampling from the scene texture and applying blur.
    //
    // This ensures we blur the actual background content, not a temp scene.
    //
    // Flow:
    // 1. Scene building: record blur view info (position, size, etc.)
    // 2. render_to_texture: render main scene
    // 3. Post-process: for each blur view, sample from scene texture, blur it
    // 4. Blit: draw scene, then blurred textures, then deferred children

    // Store the blurred texture for compositing in the final blit pass
    // Adjust transform to account for the padding offset
    let final_transform = local_transform * Affine::translate((-(padding as f64), -(padding as f64)));

    // Calculate source rectangle in scene coordinates for two-pass rendering
    // This will be used in the second pass to sample from the scene texture
    // Note: On macOS/iOS, Taffy Y-down needs to be converted to Vello Y-up for correct sampling
    let source_x = local_transform.as_coeffs()[4] as f32;  // translation x
    let source_y_taffy = local_transform.as_coeffs()[5] as f32;  // translation y (Taffy Y-down)

    // Get viewport height from scene transform (stored in _state)
    // For Y-down to Y-up conversion: vello_y = viewport_height - taffy_y - node_height
    // But we need viewport height which isn't directly available here
    // Instead, we'll store the Taffy Y value and let the copy code handle the conversion

    // Collect deferred children - they will be rendered after the blurred background
    let deferred_children: Vec<u32> = node.children.clone();

    // Store the source rectangle
    // On macOS/iOS: source_y_taffy is Y-down from top, so we store it directly
    // The copy code will handle platform-specific Y coordinate conversion
    log::debug!("[Blur] view_id={} source_rect=({:.1},{:.1}) size={:.1}x{:.1} parent_bg_check: y={:.1} h={:.1}",
        id, source_x, source_y_taffy, node_width, node_height,
        local_transform.as_coeffs()[5] - node_height, node_height);
    blurred_textures.push(BlurredTextureEntry {
        texture: offscreen_texture,
        width: texture_width,
        height: texture_height,
        transform: final_transform,
        opacity: node.opacity,
        overlay_color: node.color,
        border_radius: node.border_radius as f64,
        source_rect: (source_x, source_y_taffy, node_width as f32, node_height as f32),
        deferred_children,
        view_id: id,
        blur_radius: node.blur_radius,
        blur_style: node.blur_style,
    });

    // NOTE: For proper frosted glass effect, we do NOT draw the node's background
    // to the main scene. Instead, we want to blur the content BEHIND the node.
    // The blurred background will be composited later with a translucent tint.
    //
    // This ensures the frosted glass shows the blurred background, not its own color.

    // Children are deferred - don't render them here
    // They will be rendered after the blurred background is composited

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
    use vello::peniko::{Fill};
    use kurbo::{Rect as KRect, RoundedRect};

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

/// Render a deferred child (for frosted glass effect)
/// This renders children of blur views on top of the blurred background
fn render_deferred_child(
    id: u32,
    state: &SharedState,
    editors: &mut std::collections::HashMap<u32, Editor>,
    scene: &mut Scene,
    parent_pos: Vec2,
) {
    use vello::peniko::{Fill, BlendMode as PenikoBlendMode, Mix, Compose};
    use kurbo::{Rect as KRect, RoundedRect};

    if let Some(node) = state.nodes.get(&id) {
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let x = layout.location.x as f64 + node.position_x as f64;
        let y = layout.location.y as f64 + node.position_y as f64;
        let width = layout.size.width as f64;
        let height = layout.size.height as f64;

        let local_transform = Affine::translate((parent_pos.x + x, parent_pos.y + y));

        // Apply opacity using layer if needed
        let needs_layer = node.opacity < 1.0;
        if needs_layer {
            let alpha = node.opacity.clamp(0.0, 1.0);
            let blend = PenikoBlendMode::new(Mix::Normal, Compose::SrcOver);
            let rect = KRect::from_origin_size((0.0, 0.0), (width, height));
            scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &rect);
        }

        // Draw the child
        if node.view_type == ViewType::Text {
            if let Some(editor) = editors.get_mut(&id) {
                editor.set_width(None);
                editor.draw(scene, local_transform);
            }
        } else {
            let rect = KRect::from_origin_size((0.0, 0.0), (width, height));
            if node.border_radius > 0.0 {
                let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                scene.fill(Fill::NonZero, local_transform, node.color, None, &rounded);
            } else {
                scene.fill(Fill::NonZero, local_transform, node.color, None, &rect);
            }
        }

        // Pop layer if pushed
        if needs_layer {
            scene.pop_layer();
        }

        // Recursively render grandchildren
        let child_pos = parent_pos + Vec2::new(x, y);
        for &child_id in &node.children {
            render_deferred_child(child_id, state, editors, scene, child_pos);
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
) {
    use vello::peniko::{BlendMode as PenikoBlendMode, Mix, Compose, Fill};
    use kurbo::{Affine, Rect as KRect, RoundedRect};

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
        let local_transform = transform * Affine::translate((global_pos.x + pos_offset.x, global_pos.y + pos_offset.y));

        // Determine if we need layer effects
        let needs_layer = node.opacity < 1.0 || node.clip_to_bounds || node.blur_radius > 0.0;
        let has_shadow = node.shadow_blur > 0.0 && (node.shadow_offset_x != 0.0 || node.shadow_offset_y != 0.0 || node.shadow_blur > 0.0);
        let has_blur = node.blur_radius > 0.0;

        // NOTE: When blur is enabled, we skip layer creation here because:
        // 1. The node's background should NOT be drawn to the main scene
        // 2. Blur effect handles opacity and compositing separately
        let needs_layer_without_blur = needs_layer && !has_blur;

        // Debug: Log blur node info
        if has_blur {
            log::debug!("[Debug] Blur node id={} color={:?} blur_radius={} opacity={}",
                id, node.color, node.blur_radius, node.opacity);
            log::debug!("[Debug] Position: taffy=({:.1},{:.1}) global=({:.1},{:.1}) size={:.1}x{:.1}",
                taffy_x, taffy_y, global_pos.x, global_pos.y, node_width, node_height);
            log::debug!("[Debug] BEFORE check: id={} needs_layer={} has_blur={} needs_layer_without_blur={}",
                id, needs_layer, has_blur, needs_layer_without_blur);
        }

        // === Step 1: Draw Shadow (if any, using blur) ===
        // Xilem pattern: Draw shadow first, then content on top
        // NOTE: When blur is enabled, skip shadow in Pass 1. Shadow will be handled
        // by the blur compositing pipeline to avoid double-rendering.
        if has_shadow && !has_blur {
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
        // NOTE: When blur is enabled, we skip layer creation here because:
        // 1. The node's background should NOT be drawn to the main scene
        // 2. Blur effect handles opacity and compositing separately

        if needs_layer_without_blur {
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
        let blur_applied = if has_blur && filter_pipeline.is_some() {
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
            )
        } else {
            false
        };

        // === Step 4: Draw Node Content ===
        // Skip normal drawing if blur was applied (blur texture will be drawn in blit pass)
        if !blur_applied {
            if node.view_type == ViewType::Text {
                // Render text using Editor
                if let Some(editor) = editors.get_mut(&id) {
                    editor.set_width(None);
                    editor.draw(scene, local_transform);
                }
            } else {
                // Render rectangle at local position
                let rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));

                // Debug: Log fill operations for non-text nodes
                log::debug!("[DebugFill] id={} color={:?} size={}x{} transform={:?}",
                    id, node.color, node_width, node_height, local_transform);

                if node.border_radius > 0.0 {
                    let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                    scene.fill(Fill::NonZero, local_transform, node.color, None, &rounded);
                } else {
                    scene.fill(Fill::NonZero, local_transform, node.color, None, &rect);
                }
            }
        }

        // === Step 5: Recursively render children ===
        // For blur views: skip children in Pass 1, they will be rendered to
        // a separate texture in Pass 3 and composited on top of blur in blit pass.
        // For non-blur views: render children normally.
        // DEBUG: Log children traversal
        if !node.children.is_empty() {
            log::debug!("[DebugChildren] id={} has {} children: {:?}", id, node.children.len(), node.children);
        }
        if !blur_applied {
            let local_pos = global_pos + pos_offset;
            for &child_id in &node.children {
                log::debug!("[DebugChildren] id={} rendering child_id={}", id, child_id);
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
                );
            }
        }

        // === Step 6: Pop Layer (if pushed) ===
        // Only pop layer if we pushed it (when blur is NOT enabled)
        if needs_layer_without_blur {
            scene.pop_layer();
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

        // Initialize filter pipeline for blur effects
        let device_arc = std::sync::Arc::new(device.clone());
        let queue_arc = std::sync::Arc::new(unsafe { &*_queue.as_ptr::<wgpu::Queue>() }.clone());
        match filter_pipeline::FilterPipeline::new(device_arc, queue_arc) {
            Ok(pipeline) => {
                *self.filter_pipeline.lock().unwrap() = Some(pipeline);
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
                device_arc,
                texture_pool::TexturePoolConfig::default(),
            );
            *self.texture_pool.lock().unwrap() = Some(pool);
            log::info!("[TexturePool] Initialized blur texture pool");
        }

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
            log::info!("VelloBackend: VSync enabled (Fifo present mode)");
            wgpu::PresentMode::Fifo
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

        // Create children blit pipeline with alpha blending
        let children_blit_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Children Blit Pipeline"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
        *self.children_blit_pipeline.lock().unwrap() = Some(children_blit_p);

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Test coordinate transformation for blur sampling
    /// Verifies that source rectangle is calculated correctly for two-pass blur
    #[test]
    fn test_blur_source_rect_calculation() {
        // Simulate a view at position (100, 200) with size 120x120
        let local_transform = Affine::translate((100.0, 200.0));
        let node_width = 120.0;
        let node_height = 120.0;

        // Extract translation (same logic as render_with_blur)
        let mat = local_transform.as_coeffs();
        let source_x = mat[4] as f32; // translation x
        let source_y = mat[5] as f32; // translation y

        assert_eq!(source_x, 100.0);
        assert_eq!(source_y, 200.0);

        // Verify source_rect tuple
        let source_rect = (source_x, source_y, node_width as f32, node_height as f32);
        assert_eq!(source_rect, (100.0, 200.0, 120.0, 120.0));
    }

    /// Test Y-coordinate flipping for wgpu texture copy
    /// Vello uses Y-up, wgpu uses Y-down
    #[test]
    fn test_y_flip_calculation() {
        let screen_height = 800u32;
        let src_y = 200.0f32;
        let src_h = 120.0f32;

        // Flip Y coordinate: Vello Y=0 is bottom, wgpu Y=0 is top
        let flipped_y = (screen_height as f32 - src_y - src_h).max(0.0) as u32;

        // Expected: 800 - 200 - 120 = 480
        assert_eq!(flipped_y, 480);
    }

    /// Test padding calculation for blur bleed
    #[test]
    fn test_blur_padding_calculation() {
        let blur_radius = 10.0f64;
        let padding = (blur_radius * 2.5).ceil() as u32;

        // Expected: 10.0 * 2.5 = 25.0, ceil = 25
        assert_eq!(padding, 25);

        let texture_width = (120.0f64 as u32 + padding * 2).max(1);
        let texture_height = (120.0f64 as u32 + padding * 2).max(1);

        // Expected: 120 + 25*2 = 170
        assert_eq!(texture_width, 170);
        assert_eq!(texture_height, 170);
    }

    /// Test transform adjustment for padding offset
    #[test]
    fn test_transform_with_padding_offset() {
        let local_transform = Affine::translate((100.0, 200.0));
        let blur_radius = 10.0f64;
        let padding = (blur_radius * 2.5).ceil() as u32;

        // Adjust transform to account for padding offset
        let final_transform = local_transform * Affine::translate((-(padding as f64), -(padding as f64)));
        let final_mat = final_transform.as_coeffs();

        // Translation should be offset by padding
        assert_eq!(final_mat[4], 100.0 - padding as f64); // x: 100 - 25 = 75
        assert_eq!(final_mat[5], 200.0 - padding as f64); // y: 200 - 25 = 175
    }

    /// Test that padding is consistent between texture creation and transform
    #[test]
    fn test_padding_consistency() {
        let blur_radius = 10.0;
        let node_width = 120.0;
        let node_height = 120.0;

        // Calculate padding (same as render_with_blur)
        let padding = (blur_radius as f64 * 2.5).ceil() as u32;

        // Texture size with padding
        let texture_width = (node_width as u32 + padding * 2).max(1);
        let texture_height = (node_height as u32 + padding * 2).max(1);

        // Verify padding is applied equally on both sides
        let inner_width = texture_width - padding * 2;
        let inner_height = texture_height - padding * 2;

        assert_eq!(inner_width, node_width as u32);
        assert_eq!(inner_height, node_height as u32);
    }

    /// Test frosted glass color extraction
    #[test]
    fn test_frosted_glass_color() {
        // Color from layer_effects_demo.rs: (255u32, 255, 255, 180)
        let color = (255u32, 255, 255, 180);

        // Convert to f32 premultiplied (as done in blur_composite.wgsl)
        let alpha = color.3 as f32 / 255.0;
        let r = (color.0 as f32 / 255.0) * alpha;
        let g = (color.1 as f32 / 255.0) * alpha;
        let b = (color.2 as f32 / 255.0) * alpha;

        // White with 180/255 alpha should have premultiplied values
        assert!((r - 0.705).abs() < 0.01, "R should be ~0.705, got {}", r);
        assert!((g - 0.705).abs() < 0.01, "G should be ~0.705, got {}", g);
        assert!((b - 0.705).abs() < 0.01, "B should be ~0.705, got {}", b);
    }
}
