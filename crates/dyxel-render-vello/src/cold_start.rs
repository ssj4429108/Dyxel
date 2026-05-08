// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cold-start initialization: async renderer loading, cache management, and pipeline prewarming.

use super::{AsyncShared, VelloBackend};
use std::sync::atomic::AtomicBool;
use vello::peniko::Color;
use vello::{Renderer, RendererOptions, Scene};

impl VelloBackend {
    /// Async renderer initialization - non-blocking, runs in background thread
    /// Two-stage loading: Stage 1 (fast), save cache, Stage 2 (complete), update cache
    pub(crate) fn ensure_renderer_initialized_async(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        // Fast path - already initialized
        if self.renderer_state.is_renderer_ready() {
            return;
        }

        // Check if already loading
        if self.renderer_state.is_loading() {
            return;
        }

        // Try to acquire init info
        let init_info = self.renderer_state.take_deferred_init_info();
        if init_info.is_none() {
            return; // No init info available (should not happen)
        }

        let (_cache_path, pipeline_cache, cache_stage) = init_info.unwrap();

        // Defensive: if the renderer pipeline cache was never set (e.g. init raced), populate it now.
        self.renderer_state
            .restore_pipeline_cache_if_missing(&pipeline_cache);

        let memory_tier = self.renderer_state.memory_tier();

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
        self.renderer_state.set_loading(true);

        // Clone necessary data for the background thread
        let renderer_clone = self.renderer_state.renderer_handle_clone();
        let renderer_id_clone = self.renderer_state.renderer_id_handle();
        let is_loading_clone = self.renderer_state.loading_flag_handle();
        let device_clone = device.clone();
        let queue_clone = queue.clone();
        let perf_monitor_clone = self.diagnostics_state.perf_monitor_handle();
        let cache_saved_clone = std::sync::Arc::new(AtomicBool::new(false));
        let cache_saved_for_thread = cache_saved_clone.clone();
        let pipeline_cache_clone = self.renderer_state.pipeline_cache_handle();
        let cache_path_clone: AsyncShared<Option<String>> = self.renderer_state.cache_path_handle();
        let cache_stage_clone = self.renderer_state.cache_stage_handle();

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
                    renderer_id_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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
                                } else {
                                    log::error!(
                                        "[ColdStart] Failed to write Stage 1 cache to {}",
                                        path
                                    );
                                }
                            } else {
                                log::warn!("[ColdStart] Stage 1 cache get_data() returned None");
                            }
                        } else {
                            log::warn!(
                                "[ColdStart] Cannot save Stage 1 cache: cache={}, path={}",
                                cache_lock.is_some(),
                                path_lock.is_some()
                            );
                        }
                        drop(cache_lock);
                        drop(path_lock);
                    }

                    // Stage 2: If this is Stage 1 (first launch with area_only), upgrade to full in background
                    if is_first_launch && memory_tier != dyxel_perf::DeviceMemoryTier::LowEnd {
                        log::info!(
                            "[ColdStart] Starting Stage 2: Upgrading to full AA support in background"
                        );

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
                                renderer_id_clone
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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
                                        } else {
                                            log::error!(
                                                "[ColdStart] Failed to write Stage 2 cache to {}",
                                                path
                                            );
                                        }
                                    } else {
                                        log::warn!(
                                            "[ColdStart] Stage 2 cache get_data() returned None"
                                        );
                                    }
                                } else {
                                    log::warn!(
                                        "[ColdStart] Cannot save Stage 2 cache: cache={}, path={}",
                                        cache_lock.is_some(),
                                        path_lock.is_some()
                                    );
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

        self.renderer_state.set_loading_handle(handle);
    }

    /// Check if renderer is ready for rendering
    pub fn is_renderer_ready(&self) -> bool {
        self.renderer_state.is_renderer_ready()
    }

    /// Check if renderer is currently loading
    pub fn is_renderer_loading(&self) -> bool {
        self.renderer_state.is_loading()
    }

    pub(crate) fn save_cache(&self) {
        if self.renderer_state.cache_already_saved() {
            log::info!("[ColdStart] Cache already saved, skipping");
            return;
        }
        let pipeline_cache = self.renderer_state.pipeline_cache_handle();
        let cache_path = self.renderer_state.cache_path_handle();
        let cache_lock = pipeline_cache.lock().unwrap();
        let path_lock = cache_path.lock().unwrap();
        #[cfg(not(target_arch = "wasm32"))]
        let cache_stage = self.renderer_state.cache_stage_handle();
        #[cfg(not(target_arch = "wasm32"))]
        let stage_lock = cache_stage.lock().unwrap();
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
                        self.renderer_state.mark_cache_saved();
                    }
                } else {
                    log::warn!("[ColdStart] Cache get_data() returned None");
                }
            }
            #[cfg(target_arch = "wasm32")]
            let _ = (cache, path);
        } else {
            let has_cache = cache_lock.is_some();
            let has_path = path_lock.is_some();
            if !has_cache && has_path {
                log::warn!(
                    "[ColdStart] Cannot save cache: pipeline_cache object is None (PIPELINE_CACHE may not be supported by the adapter). path={}",
                    has_path
                );
            } else {
                log::warn!(
                    "[ColdStart] Cannot save cache: cache={}, path={}",
                    has_cache,
                    has_path
                );
            }
        }
    }

    /// Prewarm pipelines: create all necessary pipelines in background to reduce first-render latency
    pub(crate) fn prewarm_pipelines(&self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        log::info!("VelloBackend: Prewarming pipelines...");
        self.renderer_state.with_pipeline_cache(|pipeline_cache| {
            self.blit_state
                .prewarm_pipeline(device, format, pipeline_cache);
        });
        self.blur_state
            .ensure_blur_instanced_resources(device, format, 128);
        log::info!("VelloBackend: Pipeline prewarming complete.");
    }

    /// Ensure the blit pipeline matches the target surface format, recreating if needed.
    pub(crate) fn ensure_blit_pipeline(&self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        self.renderer_state.with_pipeline_cache(|pipeline_cache| {
            self.blit_state
                .ensure_pipeline(device, format, pipeline_cache);
        });
        self.blur_state
            .ensure_blur_instanced_resources(device, format, 128);
    }
}

/// Load pipeline cache from disk and create wgpu PipelineCache.
/// Returns `(cache_path, pipeline_cache, cache_stage)`.
#[inline]
pub(crate) fn load_pipeline_cache(
    device: &wgpu::Device,
    data_dir: &str,
) -> (String, Option<wgpu::PipelineCache>, Option<u8>) {
    let cache_path = format!("{}/vello_v1.cache", data_dir);
    log::info!("[ColdStart] Pipeline cache path: {}", cache_path);

    #[cfg(not(target_arch = "wasm32"))]
    let (cache_stage, cache_data) = match std::fs::read(&cache_path) {
        Ok(data) if data.len() > 1 => {
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
    let (cache_stage, cache_data): (Option<u8>, Option<Vec<u8>>) = (None, None);

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

    (cache_path, pipeline_cache, cache_stage)
}
