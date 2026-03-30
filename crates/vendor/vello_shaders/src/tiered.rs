// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tiered shader loading for reduced startup time
//! 
//! Shaders are divided into tiers based on when they're needed:
//! - Tier 1 (Core): Essential for basic rendering - loaded immediately
//! - Tier 2 (Path): Path preprocessing - loaded in background
//! - Tier 3 (Draw): Draw and clip operations - loaded on demand
//! - Tier 4 (MSAA): Advanced anti-aliasing - loaded when MSAA is enabled

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

/// Shader tier classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShaderTier {
    /// Core shaders needed for basic rendering (immediate load)
    Core = 1,
    /// Path preprocessing shaders (background load)
    Path = 2,
    /// Draw and clip shaders (on-demand load)
    Draw = 3,
    /// MSAA shaders (loaded when MSAA enabled)
    Msaa = 4,
}

/// Shader metadata for tiered loading
#[derive(Debug, Clone)]
pub struct TieredShaderInfo {
    pub name: String,
    pub tier: ShaderTier,
    pub dependencies: Vec<String>,
}

/// Registry of shader tiers
pub struct TieredShaderRegistry {
    shaders: RwLock<HashMap<String, ShaderTier>>,
    loaded_tiers: Mutex<u8>, // Bitmask of loaded tiers
}

impl TieredShaderRegistry {
    pub fn new() -> Self {
        let mut shaders = HashMap::new();
        
        // Tier 1: Core shaders (must be loaded immediately)
        for name in ["path_count_setup", "path_count", "coarse", 
                     "path_tiling_setup", "path_tiling", 
                     "fine_area", "fine_essential"] {
            shaders.insert(name.to_string(), ShaderTier::Core);
        }
        
        // Tier 2: Path preprocessing (can be delayed)
        for name in ["pathtag_reduce", "pathtag_reduce2",
                     "pathtag_scan1", "pathtag_scan", "pathtag_scan_large",
                     "bbox_clear", "flatten"] {
            shaders.insert(name.to_string(), ShaderTier::Path);
        }
        
        // Tier 3: Draw and clip (loaded when needed)
        for name in ["draw_reduce", "draw_leaf",
                     "clip_reduce", "clip_leaf",
                     "binning", "tile_alloc", "backdrop"] {
            shaders.insert(name.to_string(), ShaderTier::Draw);
        }
        
        // Tier 4: MSAA (only when MSAA enabled)
        for name in ["fine_msaa8", "fine_msaa16"] {
            shaders.insert(name.to_string(), ShaderTier::Msaa);
        }
        
        Self {
            shaders: RwLock::new(shaders),
            loaded_tiers: Mutex::new(0b0001), // Tier 1 marked as needed
        }
    }
    
    /// Get the tier for a shader
    pub fn get_tier(&self, name: &str) -> Option<ShaderTier> {
        self.shaders.read().ok()?.get(name).copied()
    }
    
    /// Mark a tier as needing to be loaded
    pub fn request_tier(&self, tier: ShaderTier) {
        if let Ok(mut loaded) = self.loaded_tiers.lock() {
            *loaded |= 1 << (tier as u8);
        }
    }
    
    /// Check if a tier should be loaded
    pub fn is_tier_needed(&self, tier: ShaderTier) -> bool {
        if let Ok(loaded) = self.loaded_tiers.lock() {
            (*loaded & (1 << (tier as u8))) != 0
        } else {
            false
        }
    }
    
    /// Get shaders for a specific tier
    pub fn get_shaders_for_tier(&self, tier: ShaderTier) -> Vec<String> {
        self.shaders.read()
            .map(|shaders| {
                shaders.iter()
                    .filter(|(_, t)| **t == tier)
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for TieredShaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Lazy-loaded shader group
pub struct LazyShaderGroup<T> {
    data: RwLock<Option<T>>,
    loading: Mutex<bool>,
}

impl<T> LazyShaderGroup<T> {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(None),
            loading: Mutex::new(false),
        }
    }
    
    /// Check if data is loaded
    pub fn is_loaded(&self) -> bool {
        self.data.read().map(|d| d.is_some()).unwrap_or(false)
    }
    
    /// Check if currently loading
    pub fn is_loading(&self) -> bool {
        self.loading.lock().map(|l| *l).unwrap_or(false)
    }
    
    /// Set the data
    pub fn set(&self, data: T) {
        if let Ok(mut guard) = self.data.write() {
            *guard = Some(data);
        }
        if let Ok(mut loading) = self.loading.lock() {
            *loading = false;
        }
    }
    
    /// Get the data if loaded
    pub fn get(&self) -> Option<T> 
    where 
        T: Clone 
    {
        self.data.read().ok()?.clone()
    }
    
    /// Start loading (mark as loading)
    pub fn start_loading(&self) -> bool {
        if let Ok(mut loading) = self.loading.lock() {
            if *loading || self.is_loaded() {
                return false;
            }
            *loading = true;
            true
        } else {
            false
        }
    }
}

impl<T> Default for LazyShaderGroup<T> {
    fn default() -> Self {
        Self::new()
    }
}
