// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Staged initialization for Vello renderer
//!
//! This module provides a way to initialize Vello renderer in stages:
//! 1. Stage 1: Core shaders only (~40% of total) - allows basic rendering
//! 2. Stage 2: Full shaders - complete functionality
//!
//! This reduces initial startup time while maintaining full functionality.

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Instant;

/// Initialization stage
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitStage {
    /// Not started
    None = 0,
    /// Stage 1: Core shaders loading
    CoreLoading = 1,
    /// Stage 1 complete - basic rendering available
    CoreReady = 2,
    /// Stage 2: Full shaders loading in background
    FullLoading = 3,
    /// Stage 2 complete - all features available
    FullReady = 4,
}

/// Staged initialization controller
pub struct StagedInit {
    stage: AtomicU8,
    start_time: Instant,
}

impl StagedInit {
    pub fn new() -> Self {
        Self {
            stage: AtomicU8::new(InitStage::None as u8),
            start_time: Instant::now(),
        }
    }

    /// Get current stage
    pub fn stage(&self) -> InitStage {
        match self.stage.load(Ordering::SeqCst) {
            1 => InitStage::CoreLoading,
            2 => InitStage::CoreReady,
            3 => InitStage::FullLoading,
            4 => InitStage::FullReady,
            _ => InitStage::None,
        }
    }

    /// Check if core is ready
    pub fn is_core_ready(&self) -> bool {
        self.stage.load(Ordering::SeqCst) >= InitStage::CoreReady as u8
    }

    /// Check if fully initialized
    pub fn is_full_ready(&self) -> bool {
        self.stage.load(Ordering::SeqCst) >= InitStage::FullReady as u8
    }

    /// Advance to next stage
    pub fn advance(&self) {
        let current = self.stage.load(Ordering::SeqCst);
        if current < InitStage::FullReady as u8 {
            self.stage.store(current + 1, Ordering::SeqCst);
            log::info!(
                "[StagedInit] Advanced to {:?} after {:?}",
                self.stage(),
                self.start_time.elapsed()
            );
        }
    }

    /// Get elapsed time
    pub fn elapsed(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }
}

impl Default for StagedInit {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for staged initialization
#[derive(Debug, Clone)]
pub struct StagedInitConfig {
    /// Whether to use staged initialization
    pub enabled: bool,
    /// Delay in ms before starting stage 2 (background loading)
    pub stage2_delay_ms: u64,
    /// Whether to enable stage 2 at all
    pub enable_full: bool,
}

impl Default for StagedInitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            stage2_delay_ms: 100, // Start full load 100ms after core ready
            enable_full: true,
        }
    }
}

/// Background loader for stage 2
pub struct BackgroundLoader {
    handle: Option<thread::JoinHandle<()>>,
}

impl BackgroundLoader {
    pub fn new() -> Self {
        Self { handle: None }
    }

    /// Start background loading
    pub fn start<F>(&mut self, delay_ms: u64, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.handle = Some(thread::spawn(move || {
            thread::sleep(std::time::Duration::from_millis(delay_ms));
            f();
        }));
    }

    /// Check if loading is complete
    pub fn is_complete(&self) -> bool {
        self.handle.as_ref().map_or(true, |h| h.is_finished())
    }

    /// Wait for completion
    pub fn wait(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Default for BackgroundLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Metrics for staged initialization
#[derive(Debug, Default, Clone)]
pub struct StagedInitMetrics {
    pub core_load_time_ms: u64,
    pub full_load_time_ms: u64,
    pub total_time_ms: u64,
}

/// Simplified staged init for VelloBackend
///
/// Usage:
/// 1. Create StagedInit at startup
/// 2. Load core shaders, mark CoreReady
/// 3. Start background thread for full shaders
/// 4. Use is_core_ready() / is_full_ready() to check capabilities
pub struct StagedVelloInit {
    pub staged: Arc<StagedInit>,
    pub config: StagedInitConfig,
    loader: Mutex<BackgroundLoader>,
    metrics: Mutex<StagedInitMetrics>,
}

impl StagedVelloInit {
    pub fn new(config: StagedInitConfig) -> Self {
        Self {
            staged: Arc::new(StagedInit::new()),
            config,
            loader: Mutex::new(BackgroundLoader::new()),
            metrics: Mutex::new(StagedInitMetrics::default()),
        }
    }

    /// Record core load complete
    pub fn core_loaded(&self, elapsed_ms: u64) {
        self.staged.advance();
        if let Ok(mut m) = self.metrics.lock() {
            m.core_load_time_ms = elapsed_ms;
        }
    }

    /// Start full load in background
    pub fn start_full_load<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if !self.config.enabled || !self.config.enable_full {
            return;
        }

        self.staged.advance(); // CoreLoading -> FullLoading

        let staged = self.staged.clone();
        let start = Instant::now();

        if let Ok(mut loader) = self.loader.lock() {
            loader.start(self.config.stage2_delay_ms, move || {
                f();
                staged.advance(); // FullLoading -> FullReady
                log::info!("[StagedInit] Full load completed in {:?}", start.elapsed());
            });
        }
    }

    /// Check if core is ready
    pub fn is_core_ready(&self) -> bool {
        self.staged.is_core_ready()
    }

    /// Check if fully ready
    pub fn is_full_ready(&self) -> bool {
        self.staged.is_full_ready()
    }

    /// Get current stage
    pub fn stage(&self) -> InitStage {
        self.staged.stage()
    }

    /// Wait for full initialization
    pub fn wait_for_full(&self) {
        while !self.is_full_ready() {
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Get metrics
    pub fn metrics(&self) -> StagedInitMetrics {
        self.metrics.lock().map(|m| m.clone()).unwrap_or_default()
    }
}

impl Default for StagedVelloInit {
    fn default() -> Self {
        Self::new(StagedInitConfig::default())
    }
}
