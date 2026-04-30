// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Staged shader loader with progressive enhancement

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

use crate::minimal_shaders::{ShaderStage, StagedLoadProgress, StagedLoadingConfig};

/// State of the staged loader
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoaderState {
    Idle = 0,
    LoadingMinimal = 1,
    MinimalReady = 2,
    LoadingExtended = 3,
    ExtendedReady = 4,
    LoadingFull = 5,
    FullReady = 6,
    LoadingComplete = 7,
    CompleteReady = 8,
    Failed = 9,
}

/// Staged shader loader
pub struct StagedShaderLoader {
    config: StagedLoadingConfig,
    state: Arc<AtomicU8>,
    progress: Arc<Mutex<StagedLoadProgress>>,
    target_stage: Arc<Mutex<ShaderStage>>,
    cache_path: Option<String>,
}

impl StagedShaderLoader {
    /// Create new loader, checking for existing cache
    pub fn new(cache_path: Option<String>) -> Self {
        let has_cache = cache_path
            .as_ref()
            .map(|p| std::path::Path::new(p).exists())
            .unwrap_or(false);

        let config = StagedLoadingConfig::default();
        let target = if has_cache {
            log::info!(
                "[StagedLoader] Cache found, targeting {:?}",
                config.cached_launch_target
            );
            config.cached_launch_target
        } else {
            log::info!(
                "[StagedLoader] No cache, targeting {:?} for fast first launch",
                config.first_launch_target
            );
            config.first_launch_target
        };

        Self {
            config,
            state: Arc::new(AtomicU8::new(LoaderState::Idle as u8)),
            progress: Arc::new(Mutex::new(StagedLoadProgress::new())),
            target_stage: Arc::new(Mutex::new(target)),
            cache_path,
        }
    }

    /// Get current state
    pub fn state(&self) -> LoaderState {
        match self.state.load(Ordering::SeqCst) {
            1 => LoaderState::LoadingMinimal,
            2 => LoaderState::MinimalReady,
            3 => LoaderState::LoadingExtended,
            4 => LoaderState::ExtendedReady,
            5 => LoaderState::LoadingFull,
            6 => LoaderState::FullReady,
            7 => LoaderState::LoadingComplete,
            8 => LoaderState::CompleteReady,
            9 => LoaderState::Failed,
            _ => LoaderState::Idle,
        }
    }

    /// Check if minimal shaders are ready
    pub fn is_minimal_ready(&self) -> bool {
        let state = self.state();
        state >= LoaderState::MinimalReady && state != LoaderState::Failed
    }

    /// Check if fully loaded
    pub fn is_complete(&self) -> bool {
        self.state() == LoaderState::CompleteReady
    }

    /// Get target stage
    pub fn target_stage(&self) -> ShaderStage {
        self.target_stage
            .lock()
            .map(|t| *t)
            .unwrap_or(ShaderStage::Complete)
    }

    /// Start loading process
    pub fn start<F>(&self, mut load_callback: F)
    where
        F: FnMut(ShaderStage) -> Result<(), String> + Send + 'static,
    {
        let target = self.target_stage();
        let config = self.config.clone();
        let state = self.state.clone();
        let progress = self.progress.clone();
        let cache_path = self.cache_path.clone();

        thread::spawn(move || {
            log::info!(
                "[StagedLoader] Starting staged loading, target: {:?}",
                target
            );
            let start = Instant::now();

            // Stage 0: Minimal
            state.store(LoaderState::LoadingMinimal as u8, Ordering::SeqCst);
            log::info!("[StagedLoader] Loading minimal shaders...");

            if let Err(e) = load_callback(ShaderStage::Minimal) {
                log::error!("[StagedLoader] Failed to load minimal shaders: {}", e);
                state.store(LoaderState::Failed as u8, Ordering::SeqCst);
                return;
            }

            state.store(LoaderState::MinimalReady as u8, Ordering::SeqCst);
            if let Ok(mut p) = progress.lock() {
                p.mark_stage_complete(ShaderStage::Minimal);
            }

            let minimal_time = start.elapsed();
            log::info!("[StagedLoader] Minimal shaders ready in {:?}", minimal_time);

            // If target is Minimal, we're done
            if target == ShaderStage::Minimal {
                if let Some(path) = cache_path {
                    log::info!("[StagedLoader] Saving cache to {}", path);
                    // Cache would be saved here by the caller
                }
                return;
            }

            // Wait before continuing to next stage
            if config.background_load && config.background_delay_ms > 0 {
                thread::sleep(Duration::from_millis(config.background_delay_ms));
            }

            // Continue with remaining stages
            for stage in [
                ShaderStage::Extended,
                ShaderStage::Full,
                ShaderStage::Complete,
            ] {
                if stage as u8 > target as u8 {
                    break;
                }

                let state_val = match stage {
                    ShaderStage::Extended => LoaderState::LoadingExtended,
                    ShaderStage::Full => LoaderState::LoadingFull,
                    ShaderStage::Complete => LoaderState::LoadingComplete,
                    _ => continue,
                };

                state.store(state_val as u8, Ordering::SeqCst);
                log::info!("[StagedLoader] Loading {:?} shaders...", stage);

                let stage_start = Instant::now();
                if let Err(e) = load_callback(stage) {
                    log::error!("[StagedLoader] Failed to load {:?} shaders: {}", stage, e);
                    // Don't fail completely, just stop here
                    break;
                }

                let ready_val = match stage {
                    ShaderStage::Extended => LoaderState::ExtendedReady,
                    ShaderStage::Full => LoaderState::FullReady,
                    ShaderStage::Complete => LoaderState::CompleteReady,
                    _ => continue,
                };

                state.store(ready_val as u8, Ordering::SeqCst);
                if let Ok(mut p) = progress.lock() {
                    p.mark_stage_complete(stage);
                }

                log::info!(
                    "[StagedLoader] {:?} ready in {:?}",
                    stage,
                    stage_start.elapsed()
                );
            }

            log::info!("[StagedLoader] Total loading time: {:?}", start.elapsed());
        });
    }

    /// Wait for a specific stage
    pub fn wait_for(&self, stage: ShaderStage, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            let current = self.state();
            let ready = match stage {
                ShaderStage::Minimal => current >= LoaderState::MinimalReady,
                ShaderStage::Extended => current >= LoaderState::ExtendedReady,
                ShaderStage::Full => current >= LoaderState::FullReady,
                ShaderStage::Complete => current >= LoaderState::CompleteReady,
            };

            if ready || current == LoaderState::Failed {
                return ready;
            }

            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Get progress information
    pub fn progress(&self) -> StagedLoadProgress {
        self.progress
            .lock()
            .map(|p| StagedLoadProgress {
                current_stage: p.current_stage,
                stages_completed: p.stages_completed.clone(),
                total_shaders_loaded: p.total_shaders_loaded,
                start_time: p.start_time,
            })
            .unwrap_or_default()
    }

    /// Get estimated time remaining
    pub fn estimated_remaining_ms(&self) -> u64 {
        let progress = self.progress();
        let target = self.target_stage();

        let elapsed = progress.elapsed_ms();
        let target_time = target.estimated_time_ms();

        if elapsed >= target_time {
            0
        } else {
            target_time - elapsed
        }
    }
}

/// Builder for staged loading configuration
pub struct StagedLoaderBuilder {
    config: StagedLoadingConfig,
    cache_path: Option<String>,
}

impl StagedLoaderBuilder {
    pub fn new() -> Self {
        Self {
            config: StagedLoadingConfig::default(),
            cache_path: None,
        }
    }

    pub fn first_launch_target(mut self, stage: ShaderStage) -> Self {
        self.config.first_launch_target = stage;
        self
    }

    pub fn cached_launch_target(mut self, stage: ShaderStage) -> Self {
        self.config.cached_launch_target = stage;
        self
    }

    pub fn background_load(mut self, enabled: bool) -> Self {
        self.config.background_load = enabled;
        self
    }

    pub fn background_delay(mut self, ms: u64) -> Self {
        self.config.background_delay_ms = ms;
        self
    }

    pub fn cache_path(mut self, path: impl Into<String>) -> Self {
        self.cache_path = Some(path.into());
        self
    }

    pub fn build(self) -> StagedShaderLoader {
        StagedShaderLoader::new(self.cache_path)
    }
}

impl Default for StagedLoaderBuilder {
    fn default() -> Self {
        Self::new()
    }
}
