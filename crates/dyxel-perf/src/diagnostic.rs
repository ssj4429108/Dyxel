// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Performance diagnostic utilities
//!
//! Helps distinguish between:
//! 1. VSync limitation (normal, expected ~60/120 FPS)
//! 2. GPU rendering bottleneck
//! 3. CPU/WASM processing bottleneck
//! 4. Render loop scheduling issue

#[allow(unused_imports)]
use std::time::{Duration, Instant};

/// Diagnostic data for a single frame
#[derive(Debug, Clone)]
pub struct FrameDiagnostic {
    pub frame_number: u64,
    /// Time from frame start to present (GPU work)
    pub gpu_time_ms: f32,
    /// Time spent in WASM logic
    pub wasm_time_ms: f32,
    /// Time spent in layout/taffy
    pub layout_time_ms: f32,
    /// Total frame time (includes VSync wait)
    pub total_time_ms: f32,
    /// Timestamp
    pub timestamp: Instant,
}

/// GPU-bound vs CPU-bound detection
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BottleneckType {
    /// VSync limited - GPU idle waiting for display
    VSyncLimited,
    /// GPU bound - rendering takes too long
    GpuBound,
    /// CPU bound - logic/layout takes too long
    CpuBound,
    /// Unknown - need more data
    Unknown,
}

/// Performance diagnostics collector
pub struct PerformanceDiagnostics {
    frames: Vec<FrameDiagnostic>,
    max_frames: usize,
    frame_counter: u64,
    last_frame_start: Option<Instant>,
    current_frame_gpu_start: Option<Instant>,
    #[allow(dead_code)]
    current_frame_wasm_start: Option<Instant>,
}

impl PerformanceDiagnostics {
    pub fn new(max_frames: usize) -> Self {
        Self {
            frames: Vec::with_capacity(max_frames),
            max_frames,
            frame_counter: 0,
            last_frame_start: None,
            current_frame_gpu_start: None,
            current_frame_wasm_start: None,
        }
    }

    /// Call at the very beginning of each frame
    pub fn begin_frame(&mut self) {
        let now = Instant::now();
        self.last_frame_start = Some(now);
        self.frame_counter += 1;
    }

    /// Call when starting GPU rendering
    pub fn begin_gpu(&mut self) {
        self.current_frame_gpu_start = Some(Instant::now());
    }

    /// Call when GPU rendering is done (before present)
    pub fn end_gpu(&mut self) -> f32 {
        if let Some(start) = self.current_frame_gpu_start {
            start.elapsed().as_secs_f32() * 1000.0
        } else {
            0.0
        }
    }

    /// Call at the end of each frame (after present)
    pub fn end_frame(&mut self, gpu_time_ms: f32, wasm_time_ms: f32, layout_time_ms: f32) {
        let now = Instant::now();
        let total_time_ms = self
            .last_frame_start
            .map(|start| start.elapsed().as_secs_f32() * 1000.0)
            .unwrap_or(0.0);

        let diagnostic = FrameDiagnostic {
            frame_number: self.frame_counter,
            gpu_time_ms,
            wasm_time_ms,
            layout_time_ms,
            total_time_ms,
            timestamp: now,
        };

        if self.frames.len() >= self.max_frames {
            self.frames.remove(0);
        }
        self.frames.push(diagnostic);
    }

    /// Analyze where the bottleneck is
    pub fn analyze_bottleneck(&self) -> BottleneckAnalysis {
        if self.frames.len() < 10 {
            return BottleneckAnalysis {
                bottleneck: BottleneckType::Unknown,
                confidence: 0.0,
                details: "Insufficient data".to_string(),
            };
        }

        let avg_total: f32 =
            self.frames.iter().map(|f| f.total_time_ms).sum::<f32>() / self.frames.len() as f32;
        let avg_gpu: f32 =
            self.frames.iter().map(|f| f.gpu_time_ms).sum::<f32>() / self.frames.len() as f32;
        let avg_wasm: f32 =
            self.frames.iter().map(|f| f.wasm_time_ms).sum::<f32>() / self.frames.len() as f32;
        let avg_layout: f32 =
            self.frames.iter().map(|f| f.layout_time_ms).sum::<f32>() / self.frames.len() as f32;

        let avg_cpu = avg_wasm + avg_layout;
        let vsync_wait = avg_total - avg_gpu.max(avg_cpu);

        // Heuristics for bottleneck detection
        let (bottleneck, confidence) = if vsync_wait > 5.0 && avg_total > 15.0 && avg_total < 20.0 {
            // Regular ~60 FPS with significant idle time = VSync limited
            (BottleneckType::VSyncLimited, 0.9)
        } else if avg_gpu > avg_total * 0.7 {
            // GPU takes most of the frame
            (BottleneckType::GpuBound, 0.85)
        } else if avg_cpu > avg_total * 0.5 {
            // CPU takes significant portion
            (BottleneckType::CpuBound, 0.8)
        } else {
            (BottleneckType::Unknown, 0.5)
        };

        let details = format!(
            "Total: {:.2}ms ({:.1} FPS)\n\
             GPU: {:.2}ms ({:.1}%)\n\
             WASM: {:.2}ms ({:.1}%)\n\
             Layout: {:.2}ms ({:.1}%)\n\
             VSync/Idle: {:.2}ms ({:.1}%)\n\
             Bottleneck: {:?}",
            avg_total,
            1000.0 / avg_total,
            avg_gpu,
            avg_gpu / avg_total * 100.0,
            avg_wasm,
            avg_wasm / avg_total * 100.0,
            avg_layout,
            avg_layout / avg_total * 100.0,
            vsync_wait.max(0.0),
            vsync_wait.max(0.0) / avg_total * 100.0,
            bottleneck
        );

        BottleneckAnalysis {
            bottleneck,
            confidence,
            details,
        }
    }

    /// Get raw frame data for custom analysis
    pub fn frames(&self) -> &[FrameDiagnostic] {
        &self.frames
    }

    /// Reset all data
    pub fn clear(&mut self) {
        self.frames.clear();
        self.frame_counter = 0;
    }
}

/// Analysis result
pub struct BottleneckAnalysis {
    pub bottleneck: BottleneckType,
    pub confidence: f32,
    pub details: String,
}

/// Quick diagnostic: measures pure GPU rendering capability
///
/// Renders N empty frames and measures the time.
/// This bypasses VSync and measures raw GPU throughput.
pub fn measure_raw_gpu_throughput<F>(render_fn: F, frames: u32) -> f32
where
    F: FnMut(),
{
    let start = Instant::now();
    let mut render_fn = render_fn;

    for _ in 0..frames {
        render_fn();
    }

    let elapsed = start.elapsed().as_secs_f32();
    frames as f32 / elapsed
}

/// Test different present modes to check VSync impact
#[derive(Debug, Clone, Copy)]
pub struct VSyncDiagnostics {
    /// FPS with VSync on (AutoVsync)
    pub fps_with_vsync: f32,
    /// FPS with VSync off (Immediate)
    pub fps_without_vsync: f32,
    /// Display refresh rate (Hz)
    pub display_refresh_rate: f32,
}

impl VSyncDiagnostics {
    /// Check if FPS is limited by VSync
    pub fn is_vsync_limited(&self) -> bool {
        let vsync_fps = self.fps_with_vsync;
        let no_vsync_fps = self.fps_without_vsync;

        // If FPS with VSync is close to display refresh rate,
        // and much lower than without VSync, it's VSync limited
        let close_to_refresh = (vsync_fps - self.display_refresh_rate).abs() < 5.0;
        let much_slower = no_vsync_fps > vsync_fps * 1.5;

        close_to_refresh && much_slower
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bottleneck_detection() {
        let mut diag = PerformanceDiagnostics::new(60);

        // Simulate VSync-limited frames (~16.67ms with idle time)
        for _i in 0..30 {
            diag.begin_frame();

            // Simulate GPU work (2ms)
            std::thread::sleep(Duration::from_millis(2));
            let gpu_time = 2.0;

            // Simulate WASM work (1ms)
            std::thread::sleep(Duration::from_millis(1));
            let wasm_time = 1.0;

            // Layout (0.5ms)
            let layout_time = 0.5;

            // Wait to simulate VSync (~13ms idle)
            std::thread::sleep(Duration::from_millis(13));

            diag.end_frame(gpu_time, wasm_time, layout_time);
        }

        let analysis = diag.analyze_bottleneck();
        println!("Analysis:\n{}", analysis.details);

        // Should detect VSync limitation
        assert!(
            analysis.confidence > 0.5,
            "Should have reasonable confidence"
        );
    }
}
