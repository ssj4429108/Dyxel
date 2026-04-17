// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Memory optimization and tiered configuration for different device classes

use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_os = "android")]
extern "C" {
    fn __system_property_get(name: *const libc::c_char, value: *mut libc::c_char) -> i32;
}

/// Device memory tier classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceMemoryTier {
    /// High-end devices (8GB+ RAM) - Full performance
    HighEnd,
    /// Mid-range devices (4-6GB RAM) - Balanced
    MidRange,
    /// Low-end devices (<4GB RAM) - Memory constrained
    LowEnd,
}

impl DeviceMemoryTier {
    /// Auto-detect based on available memory
    /// Can be overridden by DYXEL_FORCE_TIER env var for testing
    pub fn auto_detect() -> Self {
        // Check for environment variable override (for testing)
        if let Ok(tier_str) = std::env::var("DYXEL_FORCE_TIER") {
            let tier = match tier_str.to_lowercase().as_str() {
                "low" | "lowend" | "0" => {
                    log::info!("[MemoryOptimizer] Tier forced to LowEnd via DYXEL_FORCE_TIER");
                    DeviceMemoryTier::LowEnd
                }
                "mid" | "midrange" | "1" => {
                    log::info!("[MemoryOptimizer] Tier forced to MidRange via DYXEL_FORCE_TIER");
                    DeviceMemoryTier::MidRange
                }
                "high" | "highend" | "2" => {
                    log::info!("[MemoryOptimizer] Tier forced to HighEnd via DYXEL_FORCE_TIER");
                    DeviceMemoryTier::HighEnd
                }
                _ => {
                    log::warn!(
                        "[MemoryOptimizer] Unknown DYXEL_FORCE_TIER value: {}, using auto-detect",
                        tier_str
                    );
                    Self::auto_detect_internal()
                }
            };
            return tier;
        }
        Self::auto_detect_internal()
    }

    fn auto_detect_internal() -> Self {
        #[cfg(target_os = "android")]
        {
            // Check Android system property for tier override (for testing)
            if let Some(tier_str) = Self::get_android_property("debug.dyxel.force_tier") {
                let tier_str_lower = tier_str.to_lowercase();
                match tier_str_lower.as_str() {
                    "low" | "lowend" | "0" => {
                        log::info!(
                            "[MemoryOptimizer] Tier forced to LowEnd via debug.dyxel.force_tier"
                        );
                        return DeviceMemoryTier::LowEnd;
                    }
                    "mid" | "midrange" | "1" => {
                        log::info!(
                            "[MemoryOptimizer] Tier forced to MidRange via debug.dyxel.force_tier"
                        );
                        return DeviceMemoryTier::MidRange;
                    }
                    "high" | "highend" | "2" => {
                        log::info!(
                            "[MemoryOptimizer] Tier forced to HighEnd via debug.dyxel.force_tier"
                        );
                        return DeviceMemoryTier::HighEnd;
                    }
                    _ => {
                        log::warn!("[MemoryOptimizer] Unknown debug.dyxel.force_tier value: '{}', using auto-detect", tier_str);
                    }
                };
            }

            // Read total RAM from /proc/meminfo
            if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
                for line in content.lines() {
                    if line.starts_with("MemTotal:") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if let Ok(kb) = parts.get(1).unwrap_or(&"0").parse::<u64>() {
                            let mb = kb / 1024;
                            return match mb {
                                0..=4096 => DeviceMemoryTier::LowEnd,
                                4097..=6144 => DeviceMemoryTier::MidRange,
                                _ => DeviceMemoryTier::HighEnd,
                            };
                        }
                    }
                }
            }
            // Fallback if /proc/meminfo cannot be read
            return DeviceMemoryTier::MidRange;
        }

        #[cfg(target_os = "macos")]
        {
            // macOS typically has plenty of RAM
            return DeviceMemoryTier::HighEnd;
        }

        #[cfg(not(any(target_os = "android", target_os = "macos")))]
        {
            // Default to mid-range for unknown platforms (Web, etc.)
            return DeviceMemoryTier::MidRange;
        }
    }

    /// Read Android system property (internal helper)
    #[cfg(target_os = "android")]
    fn get_android_property(name: &str) -> Option<String> {
        const PROP_VALUE_MAX: usize = 92;
        let c_name = std::ffi::CString::new(name).ok()?;
        let mut buf = vec![0u8; PROP_VALUE_MAX];

        unsafe {
            let len = __system_property_get(
                c_name.as_ptr() as *const libc::c_char,
                buf.as_mut_ptr() as *mut libc::c_char,
            );
            if len > 0 {
                buf.truncate(len as usize);
                String::from_utf8(buf).ok()
            } else {
                None
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    #[allow(dead_code)]
    fn get_android_property(_name: &str) -> Option<String> {
        None
    }

    /// Get Vello renderer buffer size multiplier
    /// Balanced configuration for ~550MB memory target
    pub fn vello_buffer_multiplier(&self) -> f32 {
        match self {
            DeviceMemoryTier::HighEnd => 0.8, // 80% - optimal balance
            DeviceMemoryTier::MidRange => 0.6,
            DeviceMemoryTier::LowEnd => 0.35,
        }
    }

    /// Get maximum texture atlas size
    /// Balanced configuration
    pub fn max_atlas_size(&self) -> u32 {
        match self {
            DeviceMemoryTier::HighEnd => 2048, // 2K - good quality
            DeviceMemoryTier::MidRange => 2048,
            DeviceMemoryTier::LowEnd => 1024,
        }
    }

    /// Get font cache size limit (in MB)
    /// Balanced configuration
    pub fn font_cache_limit_mb(&self) -> usize {
        match self {
            DeviceMemoryTier::HighEnd => 96, // 96MB - sufficient for most UIs
            DeviceMemoryTier::MidRange => 64,
            DeviceMemoryTier::LowEnd => 32,
        }
    }

    /// Get WASM initial memory (in pages, 64KB each)
    /// Standard configuration
    pub fn wasm_initial_memory_pages(&self) -> u32 {
        match self {
            DeviceMemoryTier::HighEnd => 512, // 32MB
            DeviceMemoryTier::MidRange => 256,
            DeviceMemoryTier::LowEnd => 128,
        }
    }

    /// Get maximum node count before aggressive culling
    pub fn max_node_count(&self) -> usize {
        match self {
            DeviceMemoryTier::HighEnd => 10000,
            DeviceMemoryTier::MidRange => 5000,
            DeviceMemoryTier::LowEnd => 2000,
        }
    }

    /// Enable aggressive memory reclaiming
    pub fn aggressive_reclaim(&self) -> bool {
        matches!(self, DeviceMemoryTier::LowEnd)
    }

    /// Get recommended surface texture format
    /// Note: Vello internal storage requires Rgba8Unorm, this is for swapchain format only
    pub fn preferred_surface_format(&self) -> Option<&'static str> {
        match self {
            // High-end devices can use higher precision surface formats if available
            DeviceMemoryTier::HighEnd => Some("Bgra8UnormSrgb"),
            // Mid/Low use standard format
            _ => Some("Bgra8Unorm"),
        }
    }
}

/// Global memory configuration (set at startup)
static DEVICE_TIER: std::sync::OnceLock<DeviceMemoryTier> = std::sync::OnceLock::new();

/// Memory optimizer for managing tiered memory configurations
#[derive(Debug, Clone, Copy)]
pub struct MemoryOptimizer {
    tier: DeviceMemoryTier,
}

impl MemoryOptimizer {
    /// Create a new memory optimizer with auto-detected device tier
    pub fn new() -> Self {
        let tier = DeviceMemoryTier::auto_detect();
        Self { tier }
    }

    /// Create with a specific tier (for testing)
    pub fn with_tier(tier: DeviceMemoryTier) -> Self {
        Self { tier }
    }

    /// Get the current device tier
    pub fn tier(&self) -> DeviceMemoryTier {
        self.tier
    }

    /// Initialize the optimizer (called when device is ready)
    /// Logs the detected tier and all configuration values
    pub fn initialize(&self) {
        log::info!("[MemoryOptimizer] ===== Memory Configuration =====");
        log::info!("[MemoryOptimizer] Tier: {:?}", self.tier);
        log::info!(
            "[MemoryOptimizer] Vello buffer: {:.0}%",
            self.vello_buffer_multiplier() * 100.0
        );
        log::info!(
            "[MemoryOptimizer] Max atlas size: {}",
            self.max_atlas_size()
        );
        log::info!(
            "[MemoryOptimizer] Font cache: {}MB",
            self.font_cache_limit_mb()
        );
        log::info!(
            "[MemoryOptimizer] WASM initial: {} pages ({}MB)",
            self.wasm_initial_memory_pages(),
            self.wasm_initial_memory_pages() * 64 / 1024
        );
        log::info!(
            "[MemoryOptimizer] Aggressive reclaim: {}",
            self.aggressive_reclaim()
        );
        log::info!("[MemoryOptimizer] ==================================");
    }

    /// Get Vello buffer multiplier for this tier
    pub fn vello_buffer_multiplier(&self) -> f32 {
        self.tier.vello_buffer_multiplier()
    }

    /// Get maximum atlas size for this tier
    pub fn max_atlas_size(&self) -> u32 {
        self.tier.max_atlas_size()
    }

    /// Get font cache limit in MB
    pub fn font_cache_limit_mb(&self) -> usize {
        self.tier.font_cache_limit_mb()
    }

    /// Get WASM initial memory pages
    pub fn wasm_initial_memory_pages(&self) -> u32 {
        self.tier.wasm_initial_memory_pages()
    }

    /// Check if aggressive reclaim should be enabled
    pub fn aggressive_reclaim(&self) -> bool {
        self.tier.aggressive_reclaim()
    }

    /// Get recommended surface format (for swapchain configuration)
    pub fn preferred_surface_format(&self) -> Option<&'static str> {
        self.tier.preferred_surface_format()
    }
}

impl Default for MemoryOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init_device_tier() {
    let tier = DeviceMemoryTier::auto_detect();
    log::info!("[MemoryOptimizer] Device tier detected: {:?}", tier);
    log::info!("[MemoryOptimizer] Config - Vello buffer: {:.0}%, Atlas: {}, Font cache: {}MB, WASM init: {}pages",
        tier.vello_buffer_multiplier() * 100.0,
        tier.max_atlas_size(),
        tier.font_cache_limit_mb(),
        tier.wasm_initial_memory_pages()
    );
    let _ = DEVICE_TIER.set(tier);
}

pub fn get_device_tier() -> DeviceMemoryTier {
    *DEVICE_TIER.get().unwrap_or(&DeviceMemoryTier::MidRange)
}

/// Memory pressure monitoring
pub struct MemoryPressureMonitor {
    last_memory_mb: AtomicUsize,
    pressure_callbacks: Vec<Box<dyn Fn(MemoryPressureLevel) + Send + Sync>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPressureLevel {
    Normal,
    Warning,  // >70% usage
    Critical, // >85% usage
}

impl MemoryPressureMonitor {
    pub fn new() -> Self {
        Self {
            last_memory_mb: AtomicUsize::new(0),
            pressure_callbacks: Vec::new(),
        }
    }

    pub fn check_pressure(&self, current_mb: usize) -> MemoryPressureLevel {
        let tier = get_device_tier();
        let limit_mb = match tier {
            DeviceMemoryTier::HighEnd => 800,
            DeviceMemoryTier::MidRange => 500,
            DeviceMemoryTier::LowEnd => 350,
        };

        let usage_ratio = current_mb as f32 / limit_mb as f32;

        let level = if usage_ratio > 0.85 {
            MemoryPressureLevel::Critical
        } else if usage_ratio > 0.70 {
            MemoryPressureLevel::Warning
        } else {
            MemoryPressureLevel::Normal
        };

        self.last_memory_mb.store(current_mb, Ordering::Relaxed);
        level
    }

    pub fn on_pressure<F>(&mut self, callback: F)
    where
        F: Fn(MemoryPressureLevel) + Send + Sync + 'static,
    {
        self.pressure_callbacks.push(Box::new(callback));
    }
}

impl Default for MemoryPressureMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Dynamic buffer size calculator
pub struct DynamicBufferSize {
    node_count: usize,
    texture_count: usize,
}

impl DynamicBufferSize {
    pub fn new(node_count: usize, texture_count: usize) -> Self {
        Self {
            node_count,
            texture_count,
        }
    }

    /// Calculate optimal buffer size based on content complexity
    pub fn calculate_render_buffer_size(&self) -> usize {
        let tier = get_device_tier();
        let base_size = 16 * 1024 * 1024; // 16MB base

        // Scale with node count
        let node_multiplier = (self.node_count as f32 / 1000.0).max(0.5).min(3.0);

        let size = (base_size as f32 * node_multiplier * tier.vello_buffer_multiplier()) as usize;

        // Clamp to reasonable bounds
        size.min(64 * 1024 * 1024).max(4 * 1024 * 1024)
    }

    /// Calculate optimal atlas size
    pub fn calculate_atlas_size(&self) -> u32 {
        let tier = get_device_tier();
        let max_size = tier.max_atlas_size();

        // Scale with texture count
        if self.texture_count < 10 {
            max_size / 2
        } else if self.texture_count < 50 {
            max_size
        } else {
            max_size
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_tier_detection() {
        // This will run on current device
        let tier = DeviceMemoryTier::auto_detect();
        println!("Detected tier: {:?}", tier);

        // Verify tier-specific values are reasonable
        assert!(tier.vello_buffer_multiplier() > 0.0 && tier.vello_buffer_multiplier() <= 1.0);
        assert!(tier.max_atlas_size() >= 1024);
        assert!(tier.font_cache_limit_mb() >= 32);
    }

    #[test]
    fn test_dynamic_buffer_calculation() {
        init_device_tier();

        let buffer = DynamicBufferSize::new(100, 5);
        let size = buffer.calculate_render_buffer_size();

        assert!(size >= 4 * 1024 * 1024); // At least 4MB
        assert!(size <= 64 * 1024 * 1024); // At most 64MB
    }
}
