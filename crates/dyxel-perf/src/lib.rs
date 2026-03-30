// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Performance monitoring for Dyxel framework
//!
//! Provides FPS tracking, CPU and memory usage monitoring with platform-specific
//! backends for Android, macOS, and Web.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod platform;
pub use platform::create_system_info_provider;

pub mod diagnostic;
pub use diagnostic::*;

pub mod detailed_diag;
pub use detailed_diag::*;

pub mod memory_optimizer;
pub use memory_optimizer::*;

/// Performance statistics for a single frame
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStats {
    /// Frames per second (sliding average)
    pub fps: f32,
    /// Frame time in milliseconds
    pub frame_time_ms: f32,
    /// CPU usage percentage (0-100)
    pub cpu_usage: f32,
    /// Memory used in MB
    pub memory_used_mb: f32,
    /// Memory available in MB (optional)
    pub memory_available_mb: Option<f32>,
    /// CPU temperature in Celsius (if available)
    pub temperature_c: Option<f32>,
    /// Total frames rendered
    pub total_frames: u64,
    /// Timestamp of this stats sample
    pub timestamp_ms: u64,
}

/// Performance monitor configuration
#[derive(Debug, Clone, Copy)]
pub struct PerfConfig {
    /// Enable FPS monitoring
    pub enable_fps: bool,
    /// Enable CPU monitoring
    pub enable_cpu: bool,
    /// Enable memory monitoring
    pub enable_memory: bool,
    /// Enable debug overlay display
    pub enable_overlay: bool,
    /// FPS sample window size (number of frames for averaging)
    pub fps_sample_count: usize,
    /// CPU/Memory update interval in milliseconds
    pub system_sample_interval_ms: u64,
    /// Overlay position (x, y in pixels from top-left)
    pub overlay_position: (f32, f32),
    /// Overlay scale
    pub overlay_scale: f32,
}

impl Default for PerfConfig {
    fn default() -> Self {
        Self {
            enable_fps: true,
            enable_cpu: true,
            enable_memory: true,
            enable_overlay: false, // Off by default
            fps_sample_count: 60,
            system_sample_interval_ms: 1000,
            overlay_position: (10.0, 10.0), // Top-left corner with padding
            overlay_scale: 1.0,
        }
    }
}

/// Platform-specific system info provider trait
pub trait SystemInfoProvider: Send + Sync {
    /// Get current memory usage in bytes
    fn get_memory_usage(&self) -> Option<(u64, Option<u64>)> {
        None // (used, available) - None if not supported
    }

    /// Get current CPU usage percentage (0-100)
    fn get_cpu_usage(&self) -> Option<f32> {
        None
    }
    
    /// Get CPU temperature in Celsius (if available)
    fn get_temperature(&self) -> Option<f32> {
        None
    }

    /// Platform name
    fn platform_name(&self) -> &'static str;
}

/// No-op provider for unsupported platforms
pub struct NoopSystemInfoProvider;

impl SystemInfoProvider for NoopSystemInfoProvider {
    fn platform_name(&self) -> &'static str {
        "unknown"
    }
}

/// Ring buffer for frame time history
pub struct FrameTimeBuffer {
    buffer: Vec<f32>,
    index: usize,
    capacity: usize,
}

impl FrameTimeBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
            index: 0,
            capacity,
        }
    }

    pub fn push(&mut self, frame_time_ms: f32) {
        if self.buffer.len() < self.capacity {
            self.buffer.push(frame_time_ms);
        } else {
            self.buffer[self.index] = frame_time_ms;
            self.index = (self.index + 1) % self.capacity;
        }
    }

    pub fn average(&self) -> f32 {
        if self.buffer.is_empty() {
            return 0.0;
        }
        self.buffer.iter().sum::<f32>() / self.buffer.len() as f32
    }

    pub fn fps(&self) -> f32 {
        let avg = self.average();
        if avg > 0.0 {
            1000.0 / avg
        } else {
            0.0
        }
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

/// Memory usage history entry
#[derive(Debug, Clone)]
pub struct MemoryHistoryEntry {
    pub timestamp_ms: u64,
    pub memory_mb: f32,
}

/// Performance monitor for tracking FPS, CPU, and memory usage
pub struct PerformanceMonitor {
    config: PerfConfig,
    frame_buffer: Mutex<FrameTimeBuffer>,
    last_frame_time: Mutex<Instant>,
    system_info: Arc<dyn SystemInfoProvider>,
    cached_stats: Mutex<FrameStats>,
    last_system_sample: Mutex<Instant>,
    total_frames: Mutex<u64>,
    // Memory leak detection
    memory_history: Mutex<Vec<MemoryHistoryEntry>>,
    max_memory_history: usize,
    // Async renderer startup time
    startup_time_ms: Mutex<f32>,
}

impl PerformanceMonitor {
    /// Create a new performance monitor with the given configuration
    pub fn new(config: PerfConfig) -> Self {
        let system_info = create_system_info_provider();

        Self {
            config,
            frame_buffer: Mutex::new(FrameTimeBuffer::new(config.fps_sample_count)),
            last_frame_time: Mutex::new(Instant::now()),
            system_info,
            cached_stats: Mutex::new(FrameStats::default()),
            last_system_sample: Mutex::new(Instant::now() - Duration::from_secs(10)), // Force first sample
            total_frames: Mutex::new(0),
            memory_history: Mutex::new(Vec::with_capacity(300)), // 5 minutes at 1 sample/sec
            max_memory_history: 300,
            startup_time_ms: Mutex::new(0.0),
        }
    }

    /// Create with default config (overlay disabled)
    pub fn default() -> Self {
        Self::new(PerfConfig::default())
    }

    /// Create with overlay enabled
    pub fn with_overlay() -> Self {
        let mut config = PerfConfig::default();
        config.enable_overlay = true;
        Self::new(config)
    }

    /// Called at the beginning of each frame render
    pub fn begin_frame(&self) {
        if !self.config.enable_fps {
            return;
        }

        let now = Instant::now();
        let mut last_time = self.last_frame_time.lock().unwrap();

        let frame_time = now.duration_since(*last_time);
        let frame_time_ms = frame_time.as_secs_f32() * 1000.0;

        self.frame_buffer.lock().unwrap().push(frame_time_ms);
        *last_time = now;

        *self.total_frames.lock().unwrap() += 1;
    }
    
    /// Record async renderer startup time
    pub fn record_startup_time(&self, duration: Duration) {
        let ms = duration.as_secs_f32() * 1000.0;
        *self.startup_time_ms.lock().unwrap() = ms;
        log::info!("[PerfMonitor] Async renderer startup time: {:.2}ms", ms);
    }
    
    /// Get the recorded startup time in milliseconds
    pub fn get_startup_time_ms(&self) -> f32 {
        *self.startup_time_ms.lock().unwrap()
    }

    /// Get current frame statistics
    pub fn get_stats(&self) -> FrameStats {
        let mut stats = if self.config.enable_fps {
            let buffer = self.frame_buffer.lock().unwrap();
            FrameStats {
                fps: buffer.fps(),
                frame_time_ms: buffer.average(),
                total_frames: *self.total_frames.lock().unwrap(),
                ..Default::default()
            }
        } else {
            FrameStats::default()
        };

        // Update system stats (CPU/Memory) only at intervals
        let now = Instant::now();
        let should_sample = {
            let last = self.last_system_sample.lock().unwrap();
            now.duration_since(*last).as_millis() as u64 >= self.config.system_sample_interval_ms
        };

        if should_sample {
            *self.last_system_sample.lock().unwrap() = now;

            if self.config.enable_memory {
                if let Some((used, available)) = self.system_info.get_memory_usage() {
                    stats.memory_used_mb = used as f32 / (1024.0 * 1024.0);
                    stats.memory_available_mb = available.map(|a| a as f32 / (1024.0 * 1024.0));
                }
            }

            if self.config.enable_cpu {
                if let Some(cpu) = self.system_info.get_cpu_usage() {
                    stats.cpu_usage = cpu;
                }
            }
            
            // Get temperature (if available)
            if let Some(temp) = self.system_info.get_temperature() {
                stats.temperature_c = Some(temp);
            }

            // Cache the system stats for subsequent calls
            let mut cached = self.cached_stats.lock().unwrap();
            cached.memory_used_mb = stats.memory_used_mb;
            cached.memory_available_mb = stats.memory_available_mb;
            cached.cpu_usage = stats.cpu_usage;
            
            // Record memory history for leak detection
            if self.config.enable_memory && stats.memory_used_mb > 0.0 {
                let mut history = self.memory_history.lock().unwrap();
                history.push(MemoryHistoryEntry {
                    timestamp_ms: stats.timestamp_ms,
                    memory_mb: stats.memory_used_mb,
                });
                if history.len() > self.max_memory_history {
                    history.remove(0);
                }
            }
        } else {
            // Use cached system stats
            let cached = self.cached_stats.lock().unwrap();
            stats.memory_used_mb = cached.memory_used_mb;
            stats.memory_available_mb = cached.memory_available_mb;
            stats.cpu_usage = cached.cpu_usage;
        }

        stats.timestamp_ms = now.elapsed().as_millis() as u64;
        stats
    }
    
    /// Get memory usage history for leak detection
    pub fn get_memory_history(&self) -> Vec<MemoryHistoryEntry> {
        self.memory_history.lock().unwrap().clone()
    }
    
    /// Analyze memory trend (returns MB per minute)
    pub fn get_memory_trend(&self) -> f32 {
        let history = self.memory_history.lock().unwrap();
        if history.len() < 2 {
            return 0.0;
        }
        
        let first = &history[0];
        let last = &history[history.len() - 1];
        let duration_min = (last.timestamp_ms - first.timestamp_ms) as f32 / 60000.0;
        if duration_min <= 0.0 {
            return 0.0;
        }
        
        (last.memory_mb - first.memory_mb) / duration_min
    }
    
    /// Check for potential memory leak (>10MB/min growth)
    pub fn has_memory_leak(&self) -> bool {
        self.get_memory_trend() > 10.0
    }

    /// Check if overlay should be displayed
    pub fn should_show_overlay(&self) -> bool {
        self.config.enable_overlay
    }

    /// Get overlay configuration (x, y, scale)
    pub fn get_overlay_config(&self) -> (f32, f32, f32) {
        let (x, y) = self.config.overlay_position;
        (x, y, self.config.overlay_scale)
    }

    /// Update configuration
    pub fn set_config(&mut self, config: PerfConfig) {
        if config.fps_sample_count != self.config.fps_sample_count {
            *self.frame_buffer.lock().unwrap() = FrameTimeBuffer::new(config.fps_sample_count);
        }
        self.config = config;
    }

    /// Toggle overlay display
    pub fn toggle_overlay(&mut self) {
        self.config.enable_overlay = !self.config.enable_overlay;
    }

    /// Get platform name
    pub fn platform_name(&self) -> &'static str {
        self.system_info.platform_name()
    }
}

/// Thread-safe wrapper for sharing across threads
pub type SharedPerfMonitor = Arc<Mutex<PerformanceMonitor>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_time_buffer() {
        let mut buffer = FrameTimeBuffer::new(3);
        buffer.push(16.0);
        buffer.push(17.0);
        buffer.push(18.0);

        assert_eq!(buffer.average(), 17.0);
        assert!((buffer.fps() - 58.8).abs() < 1.0); // ~58.8 FPS

        // Test wrap-around
        buffer.push(20.0);
        buffer.push(22.0);
        assert_eq!(buffer.average(), 20.0); // (18 + 20 + 22) / 3
    }

    #[test]
    fn test_perf_monitor() {
        let monitor = PerformanceMonitor::new(PerfConfig {
            enable_fps: true,
            enable_cpu: false,
            enable_memory: false,
            enable_overlay: false,
            fps_sample_count: 10,
            system_sample_interval_ms: 1000,
            overlay_position: (0.0, 0.0),
            overlay_scale: 1.0,
        });

        // Simulate some frames
        for _ in 0..5 {
            // std::thread::sleep(Duration::from_millis(16));
            monitor.begin_frame();
        }

        let stats = monitor.get_stats();
        assert!(stats.total_frames >= 5);
    }
}
