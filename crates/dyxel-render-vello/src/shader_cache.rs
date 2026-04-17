// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shader cache management for faster startup
//!
//! This module provides a pipeline cache management system that:
//! 1. Pre-warms shader cache on first run
//! 2. Persists cache to disk for subsequent runs
//! 3. Supports background cache warming

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Shader cache configuration
pub struct ShaderCacheConfig {
    /// Cache file path
    pub cache_path: PathBuf,
    /// Whether to enable cache
    pub enabled: bool,
    /// Whether to warm cache in background
    pub background_warm: bool,
}

impl Default for ShaderCacheConfig {
    fn default() -> Self {
        Self {
            cache_path: PathBuf::from("vello_cache.bin"),
            enabled: true,
            background_warm: true,
        }
    }
}

/// Shader cache manager
pub struct ShaderCacheManager {
    config: ShaderCacheConfig,
    cache_data: Option<Vec<u8>>,
    warmed: bool,
}

impl ShaderCacheManager {
    /// Create new cache manager
    pub fn new(config: ShaderCacheConfig) -> Self {
        let cache_data = if config.enabled {
            Self::load_cache(&config.cache_path)
        } else {
            None
        };

        Self {
            config,
            cache_data,
            warmed: false,
        }
    }

    /// Check if cache is available
    pub fn has_cache(&self) -> bool {
        self.cache_data.is_some()
    }

    /// Get cache data for wgpu
    pub fn get_cache_data(&self) -> Option<&[u8]> {
        self.cache_data.as_deref()
    }

    /// Load cache from disk
    fn load_cache(path: &PathBuf) -> Option<Vec<u8>> {
        match fs::read(path) {
            Ok(data) => {
                log::info!("[ShaderCache] Loaded {} bytes from {:?}", data.len(), path);
                Some(data)
            }
            Err(e) => {
                log::debug!("[ShaderCache] No cache found: {}", e);
                None
            }
        }
    }

    /// Save cache to disk
    pub fn save_cache(&self, data: &[u8]) {
        if !self.config.enabled {
            return;
        }

        let start = Instant::now();
        match fs::write(&self.config.cache_path, data) {
            Ok(_) => {
                log::info!(
                    "[ShaderCache] Saved {} bytes to {:?} in {:?}",
                    data.len(),
                    self.config.cache_path,
                    start.elapsed()
                );
            }
            Err(e) => {
                log::warn!("[ShaderCache] Failed to save cache: {}", e);
            }
        }
    }

    /// Mark as warmed
    pub fn mark_warmed(&mut self) {
        self.warmed = true;
    }

    /// Check if cache has been warmed this session
    pub fn is_warmed(&self) -> bool {
        self.warmed
    }
}

/// Statistics for shader loading
#[derive(Debug, Default, Clone)]
pub struct ShaderLoadStats {
    pub cache_hit: bool,
    pub load_time_ms: u64,
    pub shader_count: usize,
    pub cache_size_bytes: usize,
}

impl ShaderLoadStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_cache_hit(mut self) -> Self {
        self.cache_hit = true;
        self
    }

    pub fn with_load_time(mut self, ms: u64) -> Self {
        self.load_time_ms = ms;
        self
    }
}

/// Global shader cache instance (optional singleton)
static mut GLOBAL_CACHE: Option<Arc<ShaderCacheManager>> = None;

/// Initialize global shader cache
pub fn init_global_cache(config: ShaderCacheConfig) {
    unsafe {
        GLOBAL_CACHE = Some(Arc::new(ShaderCacheManager::new(config)));
    }
}

/// Get global shader cache
#[allow(static_mut_refs)]
pub fn global_cache() -> Option<Arc<ShaderCacheManager>> {
    unsafe { GLOBAL_CACHE.clone() }
}
