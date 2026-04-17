// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Two-stage initialization for optimal cache behavior
//!
//! Stage 1: Load minimal shaders (~400ms), save cache immediately
//! Stage 2: Load remaining shaders (~800ms), update cache when complete
//!
//! This ensures:
//! - First launch: Fast startup with Stage 1, cache saved
//! - Second launch: Stage 1 from cache, Stage 2 loads and saves updated cache
//! - Third launch: Full cache hit, fastest startup

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::thread;

/// Stage of initialization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InitStage {
    None = 0,
    Stage1Loading = 1,
    Stage1Complete = 2,
    Stage2Loading = 3,
    Stage2Complete = 4,
}

/// Cache metadata to track which stage was saved
const CACHE_MAGIC: &[u8] = b"DYXL";
const CACHE_VERSION: u32 = 1;

/// Cache header with stage info
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CacheHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub stage: u8, // Which stage this cache contains
    pub reserved: [u8; 3],
}

impl CacheHeader {
    pub fn new(stage: u8) -> Self {
        Self {
            magic: [b'D', b'Y', b'X', b'L'],
            version: CACHE_VERSION,
            stage,
            reserved: [0; 3],
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 8 {
            return None;
        }
        let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if &magic != CACHE_MAGIC {
            return None;
        }
        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if version != CACHE_VERSION {
            return None;
        }
        Some(Self {
            magic,
            version,
            stage: bytes.get(8).copied().unwrap_or(0),
            reserved: [0; 3],
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&self.magic);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.push(self.stage);
        bytes.extend_from_slice(&self.reserved);
        bytes
    }
}

/// Two-stage loader controller
pub struct TwoStageLoader {
    stage: Arc<AtomicU8>,
    cache_path: String,
}

impl TwoStageLoader {
    pub fn new(cache_path: String) -> Self {
        let stage = Self::detect_stage_from_cache(&cache_path);
        log::info!("[TwoStage] Current stage from cache: {:?}", stage);

        Self {
            stage: Arc::new(AtomicU8::new(stage as u8)),
            cache_path,
        }
    }

    /// Detect stage from existing cache file
    fn detect_stage_from_cache(cache_path: &str) -> InitStage {
        match std::fs::read(cache_path) {
            Ok(data) if data.len() >= 12 => {
                if let Some(header) = CacheHeader::from_bytes(&data) {
                    match header.stage {
                        2 => InitStage::Stage1Complete,
                        4 => InitStage::Stage2Complete,
                        _ => InitStage::None,
                    }
                } else {
                    InitStage::None
                }
            }
            _ => InitStage::None,
        }
    }

    /// Get current stage
    pub fn stage(&self) -> InitStage {
        match self.stage.load(Ordering::SeqCst) {
            1 => InitStage::Stage1Loading,
            2 => InitStage::Stage1Complete,
            3 => InitStage::Stage2Loading,
            4 => InitStage::Stage2Complete,
            _ => InitStage::None,
        }
    }

    /// Check if we need Stage 2 loading
    pub fn needs_stage2(&self) -> bool {
        self.stage() != InitStage::Stage2Complete
    }

    /// Check if cache exists at all
    pub fn has_any_cache(&self) -> bool {
        std::path::Path::new(&self.cache_path).exists()
    }

    /// Save cache with stage metadata
    pub fn save_cache_with_stage(&self, wgpu_cache_data: &[u8], stage: u8) {
        let header = CacheHeader::new(stage);
        let mut data = header.to_bytes();
        data.extend_from_slice(wgpu_cache_data);

        match std::fs::write(&self.cache_path, &data) {
            Ok(_) => log::info!(
                "[TwoStage] Cache saved with stage {} ({} bytes total)",
                stage,
                data.len()
            ),
            Err(e) => log::error!("[TwoStage] Failed to save cache: {}", e),
        }
    }

    /// Load cache data (without header)
    pub fn load_cache_data(&self) -> Option<Vec<u8>> {
        match std::fs::read(&self.cache_path) {
            Ok(data) if data.len() > 12 => Some(data[12..].to_vec()),
            _ => None,
        }
    }

    /// Advance to next stage
    pub fn advance_stage(&self, stage: InitStage) {
        self.stage.store(stage as u8, Ordering::SeqCst);
    }

    /// Execute two-stage loading
    ///
    /// `stage1_loader`: Function to load minimal shaders, returns cache data
    /// `stage2_loader`: Function to load remaining shaders, returns updated cache data
    pub fn execute<F1, F2>(&self, stage1_loader: F1, stage2_loader: F2)
    where
        F1: FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
        F2: FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
    {
        let current_stage = self.stage();
        let cache_path = self.cache_path.clone();
        let stage = self.stage.clone();

        thread::spawn(move || {
            match current_stage {
                InitStage::None | InitStage::Stage1Loading => {
                    // Stage 1: Load minimal shaders
                    log::info!("[TwoStage] Starting Stage 1 (minimal shaders)");
                    stage.store(InitStage::Stage1Loading as u8, Ordering::SeqCst);

                    match stage1_loader() {
                        Ok(cache_data) => {
                            // Save Stage 1 cache immediately
                            let header = CacheHeader::new(InitStage::Stage1Complete as u8);
                            let mut data = header.to_bytes();
                            data.extend_from_slice(&cache_data);
                            let _ = std::fs::write(&cache_path, &data);

                            stage.store(InitStage::Stage1Complete as u8, Ordering::SeqCst);
                            log::info!("[TwoStage] Stage 1 complete, cache saved");

                            // Continue to Stage 2 immediately
                            log::info!("[TwoStage] Starting Stage 2 (remaining shaders)");
                            stage.store(InitStage::Stage2Loading as u8, Ordering::SeqCst);

                            match stage2_loader() {
                                Ok(updated_cache) => {
                                    let header = CacheHeader::new(InitStage::Stage2Complete as u8);
                                    let mut data = header.to_bytes();
                                    data.extend_from_slice(&updated_cache);
                                    let _ = std::fs::write(&cache_path, &data);

                                    stage.store(InitStage::Stage2Complete as u8, Ordering::SeqCst);
                                    log::info!("[TwoStage] Stage 2 complete, full cache saved");
                                }
                                Err(e) => {
                                    log::error!("[TwoStage] Stage 2 failed: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("[TwoStage] Stage 1 failed: {}", e);
                        }
                    }
                }
                InitStage::Stage1Complete => {
                    // Stage 1 was done, continue with Stage 2
                    log::info!("[TwoStage] Resuming with Stage 2 (Stage 1 was cached)");
                    stage.store(InitStage::Stage2Loading as u8, Ordering::SeqCst);

                    match stage2_loader() {
                        Ok(updated_cache) => {
                            let header = CacheHeader::new(InitStage::Stage2Complete as u8);
                            let mut data = header.to_bytes();
                            data.extend_from_slice(&updated_cache);
                            let _ = std::fs::write(&cache_path, &data);

                            stage.store(InitStage::Stage2Complete as u8, Ordering::SeqCst);
                            log::info!("[TwoStage] Stage 2 complete, full cache saved");
                        }
                        Err(e) => {
                            log::error!("[TwoStage] Stage 2 failed: {}", e);
                        }
                    }
                }
                InitStage::Stage2Complete => {
                    log::info!("[TwoStage] Already at Stage 2 complete, nothing to do");
                }
                _ => {}
            }
        });
    }
}

/// Simple wrapper for VelloBackend integration
pub struct TwoStageInit {
    pub loader: Arc<TwoStageLoader>,
}

impl TwoStageInit {
    pub fn new(cache_path: String) -> Self {
        Self {
            loader: Arc::new(TwoStageLoader::new(cache_path)),
        }
    }

    /// Check if we should use minimal config for fast startup
    pub fn use_minimal_for_startup(&self) -> bool {
        let stage = self.loader.stage();
        // Use minimal if no cache or only Stage 1
        matches!(
            stage,
            InitStage::None | InitStage::Stage1Loading | InitStage::Stage1Complete
        )
    }

    /// Check if full cache is ready
    pub fn is_full_cache_ready(&self) -> bool {
        self.loader.stage() == InitStage::Stage2Complete
    }
}
