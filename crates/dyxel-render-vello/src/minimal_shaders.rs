// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Minimal shader set for fastest possible first launch
//! 
//! This module defines the minimal set of shaders needed to render
//! basic content, reducing first launch time to ~300-500ms.
//! 
//! Stages:
//! 1. Stage 0 (Minimal): Core rendering only (~30% of shaders)
//! 2. Stage 1 (Extended): Path preprocessing
//! 3. Stage 2 (Full): Draw/clip operations
//! 4. Stage 3 (Complete): MSAA and advanced features

use std::collections::HashSet;

/// Shader loading stage
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShaderStage {
    /// Stage 0: Minimal - only core rendering shaders
    /// Time: ~300-500ms
    Minimal = 0,
    /// Stage 1: Extended - add path preprocessing
    /// Time: ~+400ms (total ~700-900ms)
    Extended = 1,
    /// Stage 2: Full - add draw/clip
    /// Time: ~+400ms (total ~1.1-1.3s)
    Full = 2,
    /// Stage 3: Complete - add MSAA
    /// Time: ~+500ms (total ~1.6-1.8s)
    Complete = 3,
}

impl ShaderStage {
    /// Get all shader names for this stage
    pub fn shader_names(&self) -> &'static [&'static str] {
        match self {
            ShaderStage::Minimal => &MINIMAL_SHADERS,
            ShaderStage::Extended => &EXTENDED_SHADERS,
            ShaderStage::Full => &FULL_SHADERS,
            ShaderStage::Complete => &COMPLETE_SHADERS,
        }
    }
    
    /// Check if a shader is in this stage
    pub fn contains(&self, name: &str) -> bool {
        self.shader_set().contains(name)
    }
    
    /// Get shader set for fast lookup
    fn shader_set(&self) -> HashSet<&'static str> {
        self.shader_names().iter().cloned().collect()
    }
    
    /// Get next stage
    pub fn next(&self) -> Option<ShaderStage> {
        match self {
            ShaderStage::Minimal => Some(ShaderStage::Extended),
            ShaderStage::Extended => Some(ShaderStage::Full),
            ShaderStage::Full => Some(ShaderStage::Complete),
            ShaderStage::Complete => None,
        }
    }
    
    /// Get total estimated load time up to this stage
    pub fn estimated_time_ms(&self) -> u64 {
        match self {
            ShaderStage::Minimal => 400,
            ShaderStage::Extended => 800,
            ShaderStage::Full => 1200,
            ShaderStage::Complete => 1800,
        }
    }
}

/// Stage 0: Minimal shaders - absolute minimum for rendering
/// These are the shaders that MUST be loaded for any rendering
const MINIMAL_SHADERS: &[&str] = &[
    // Core rasterization pipeline (fine is the heaviest but essential)
    "fine_area",          // Essential: final pixel rendering
    "coarse",             // Essential: coarse rasterization
    "path_count",         // Essential: path counting
    "path_count_setup",   // Essential: path count setup
    "path_tiling",        // Essential: path tiling
    "path_tiling_setup",  // Essential: path tiling setup
];

/// Stage 1: Extended - add path preprocessing
/// These are needed for complex paths but not simple content
const EXTENDED_SHADERS: &[&str] = &[
    "pathtag_reduce",
    "pathtag_reduce2",
    "pathtag_scan1",
    "pathtag_scan",
    "pathtag_scan_large",
    "bbox_clear",
    "flatten",
];

/// Stage 2: Full - add draw and clip operations
/// These are needed for complex scenes with clips/draws
const FULL_SHADERS: &[&str] = &[
    "draw_reduce",
    "draw_leaf",
    "clip_reduce",
    "clip_leaf",
    "binning",
    "tile_alloc",
    "backdrop",
];

/// Stage 3: Complete - add MSAA
/// These are optional, only needed for high-quality AA
const COMPLETE_SHADERS: &[&str] = &[
    "fine_msaa8",
    "fine_msaa16",
];

/// Get all shaders up to and including a stage
pub fn shaders_up_to(stage: ShaderStage) -> Vec<&'static str> {
    let mut result = Vec::new();
    for s in [ShaderStage::Minimal, ShaderStage::Extended, 
              ShaderStage::Full, ShaderStage::Complete] {
        if s as u8 <= stage as u8 {
            result.extend(s.shader_names());
        }
    }
    result
}

/// Check if a shader is required for minimal rendering
pub fn is_minimal_shader(name: &str) -> bool {
    ShaderStage::Minimal.contains(name)
}

/// Get stage for a shader
pub fn shader_stage(name: &str) -> Option<ShaderStage> {
    for stage in [ShaderStage::Minimal, ShaderStage::Extended,
                  ShaderStage::Full, ShaderStage::Complete] {
        if stage.contains(name) {
            return Some(stage);
        }
    }
    None
}

/// Configuration for staged loading
#[derive(Debug, Clone)]
pub struct StagedLoadingConfig {
    /// Target stage for first launch
    pub first_launch_target: ShaderStage,
    /// Target stage for subsequent launches (with cache)
    pub cached_launch_target: ShaderStage,
    /// Whether to continue loading in background after first launch
    pub background_load: bool,
    /// Delay before background loading (ms)
    pub background_delay_ms: u64,
}

impl Default for StagedLoadingConfig {
    fn default() -> Self {
        Self {
            // First launch: only load minimal shaders for fast startup
            first_launch_target: ShaderStage::Minimal,
            // Cached launch: load everything (cache makes it fast)
            cached_launch_target: ShaderStage::Complete,
            // Continue loading in background
            background_load: true,
            // Wait 500ms after minimal ready before loading more
            background_delay_ms: 500,
        }
    }
}

/// Progress tracker for staged loading
#[derive(Debug, Default)]
pub struct StagedLoadProgress {
    pub current_stage: Option<ShaderStage>,
    pub stages_completed: Vec<ShaderStage>,
    pub total_shaders_loaded: usize,
    pub start_time: Option<std::time::Instant>,
}

impl StagedLoadProgress {
    pub fn new() -> Self {
        Self {
            start_time: Some(std::time::Instant::now()),
            ..Default::default()
        }
    }
    
    pub fn mark_stage_complete(&mut self, stage: ShaderStage) {
        self.stages_completed.push(stage);
        self.total_shaders_loaded += stage.shader_names().len();
    }
    
    pub fn elapsed_ms(&self) -> u64 {
        self.start_time
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }
    
    pub fn is_complete(&self, target: ShaderStage) -> bool {
        self.stages_completed.iter().any(|s| *s as u8 >= target as u8)
    }
}

/// Estimate time saved by using staged loading
pub fn estimate_time_savings(target_stage: ShaderStage) -> (u64, u64, f64) {
    let full_time = ShaderStage::Complete.estimated_time_ms();
    let staged_time = target_stage.estimated_time_ms();
    let saved = full_time - staged_time;
    let percent = (saved as f64 / full_time as f64) * 100.0;
    (staged_time, saved, percent)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_minimal_shaders_count() {
        assert_eq!(MINIMAL_SHADERS.len(), 6);
    }
    
    #[test]
    fn test_shaders_up_to() {
        let minimal = shaders_up_to(ShaderStage::Minimal);
        assert_eq!(minimal.len(), 6);
        
        let extended = shaders_up_to(ShaderStage::Extended);
        assert_eq!(extended.len(), 6 + 7); // Minimal + Extended
    }
    
    #[test]
    fn test_time_savings() {
        let (time, saved, pct) = estimate_time_savings(ShaderStage::Minimal);
        assert_eq!(time, 400);
        assert_eq!(saved, 1400);
        assert!(pct > 70.0); // Should save >70%
    }
}
