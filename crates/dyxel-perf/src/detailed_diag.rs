// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Detailed timing diagnostics for pinpointing 60 FPS bottleneck
//! 
//! Usage:
//! ```
//! use dyxel_perf::FrameTimer;
//! 
//! // 在关键路径埋点
//! let mut timer = FrameTimer::new();
//! 
//! timer.mark("logic_start");
//! // ... logic tick ...
//! timer.mark("logic_end");
//! 
//! timer.mark("render_start");
//! // ... render ...
//! timer.mark("render_end");
//! 
//! timer.mark("present_start");
//! // ... present ...
//! timer.mark("present_end");
//! 
//! let report = timer.report();
//! // report.spans["logic"] = 2.1ms
//! // report.spans["render"] = 3.5ms
//! // report.spans["present"] = 10.8ms (includes VSync wait)
//! ```

use std::collections::HashMap;
#[allow(unused_imports)]
use std::time::{Duration, Instant};

/// High-precision frame timer
pub struct FrameTimer {
    frame_start: Instant,
    marks: Vec<(String, Instant)>,
}

impl FrameTimer {
    pub fn new() -> Self {
        Self {
            frame_start: Instant::now(),
            marks: Vec::with_capacity(16),
        }
    }
    
    /// Record a named timestamp
    pub fn mark(&mut self, name: impl Into<String>) {
        self.marks.push((name.into(), Instant::now()));
    }
    
    /// Calculate time spans between marks
    pub fn report(&self) -> TimingReport {
        let mut spans = HashMap::new();
        
        // Calculate consecutive spans
        for i in 1..self.marks.len() {
            let (name1, t1) = &self.marks[i-1];
            let (name2, t2) = &self.marks[i];
            
            let span_name = if name1.ends_with("_start") && name2.ends_with("_end") {
                // Pair: logic_start -> logic_end = logic
                name1.trim_end_matches("_start").to_string()
            } else {
                format!("{}_to_{}", name1, name2)
            };
            
            let duration = t2.duration_since(*t1).as_secs_f64() * 1000.0;
            spans.insert(span_name, duration);
        }
        
        // Total frame time
        if let Some((_, last_time)) = self.marks.last() {
            let total = last_time.duration_since(self.frame_start).as_secs_f64() * 1000.0;
            spans.insert("total".to_string(), total);
        }
        
        TimingReport { spans, marks: self.marks.clone() }
    }
    
    /// Get raw marks for custom analysis
    pub fn marks(&self) -> &[(String, Instant)] {
        &self.marks
    }
}

impl Default for FrameTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Timing analysis result
pub struct TimingReport {
    pub spans: HashMap<String, f64>,
    pub marks: Vec<(String, Instant)>,
}

impl TimingReport {
    /// Get a specific span duration in ms
    pub fn get(&self, name: &str) -> f64 {
        self.spans.get(name).copied().unwrap_or(0.0)
    }
    
    /// Pretty print the report
    pub fn print(&self) {
        println!("=== Frame Timing Breakdown ===");
        
        // Print in order of occurrence
        for i in 1..self.marks.len() {
            let (name1, t1) = &self.marks[i-1];
            let (name2, t2) = &self.marks[i];
            let duration = t2.duration_since(*t1).as_secs_f64() * 1000.0;
            
            println!("  {:20} -> {:20}: {:.3} ms", name1, name2, duration);
        }
        
        println!("  --------------------------------");
        if let Some(total) = self.spans.get("total") {
            println!("  TOTAL FRAME TIME: {:.3} ms ({:.1} FPS)", 
                total, 1000.0 / total);
        }
        
        // Identify bottleneck
        if let Some(gpu_time) = self.spans.get("gpu_render") {
            if let Some(total) = self.spans.get("total") {
                let idle = total - gpu_time;
                if idle > 5.0 {
                    println!("  ⚠️  IDLE/WAIT TIME: {:.3} ms (likely VSync)", idle);
                }
            }
        }
    }
}

/// Per-frame detailed diagnostics aggregator
pub struct DetailedDiagnostics {
    frames: Vec<TimingReport>,
    max_frames: usize,
}

impl DetailedDiagnostics {
    pub fn new(max_frames: usize) -> Self {
        Self {
            frames: Vec::with_capacity(max_frames),
            max_frames,
        }
    }
    
    pub fn add_frame(&mut self, report: TimingReport) {
        if self.frames.len() >= self.max_frames {
            self.frames.remove(0);
        }
        self.frames.push(report);
    }
    
    /// Analyze average timing across all frames
    pub fn analyze(&self) -> HashMap<String, (f64, f64)> {
        // (name) -> (avg_ms, max_ms)
        let mut aggregated: HashMap<String, Vec<f64>> = HashMap::new();
        
        for frame in &self.frames {
            for (name, duration) in &frame.spans {
                aggregated.entry(name.clone()).or_default().push(*duration);
            }
        }
        
        aggregated
            .into_iter()
            .map(|(name, values)| {
                let avg = values.iter().sum::<f64>() / values.len() as f64;
                let max: f64 = values.iter().fold(0.0_f64, |a, b| a.max(*b));
                (name, (avg, max))
            })
            .collect()
    }
    
    /// Print comprehensive analysis
    pub fn print_analysis(&self) {
        let analysis = self.analyze();
        
        println!("\n========== DETAILED DIAGNOSTICS ({} frames) ==========", self.frames.len());
        println!("{:<25} {:>12} {:>12}", "Stage", "Avg (ms)", "Max (ms)");
        println!("{}", "-".repeat(52));
        
        // Sort by average time
        let mut sorted: Vec<_> = analysis.iter().collect();
        sorted.sort_by(|a, b| b.1.0.partial_cmp(&a.1.0).unwrap());
        
        for (name, (avg, max)) in sorted {
            if name == "total" {
                println!("{}", "-".repeat(52));
            }
            println!("{:<25} {:>12.3} {:>12.3}", name, avg, max);
        }
        
        // Detect VSync
        if let Some((total_avg, _)) = analysis.get("total") {
            if let Some((gpu_avg, _)) = analysis.get("gpu_render") {
                let idle = total_avg - gpu_avg;
                let fps = 1000.0 / total_avg;
                
                println!("\n========== BOTTLENECK ANALYSIS ==========");
                println!("Average FPS: {:.1}", fps);
                println!("GPU Render Time: {:.3} ms", gpu_avg);
                println!("Idle/Wait Time: {:.3} ms", idle);
                
                if fps > 58.0 && fps < 62.0 && idle > 5.0 {
                    println!("🔒 VSync Limited: Yes (display refresh rate locked)");
                } else if fps > 120.0 && fps < 125.0 {
                    println!("🔒 VSync Limited: Yes (120Hz display)");
                } else {
                    println!("🔓 VSync Limited: No (or VSync disabled)");
                }
                
                if *gpu_avg > 8.0 {
                    println!("⚠️  GPU Bound: Yes (rendering takes >50% of frame)");
                }
            }
        }
        
        println!("{}", "=".repeat(52));
    }
}

/// Macro for easy instrumentation
#[macro_export]
macro_rules! timed_scope {
    ($timer:expr, $name:expr) => {
        let _timer_guard = $crate::TimerGuard::new($timer, $name);
    };
}

/// RAII timer guard
pub struct TimerGuard<'a> {
    timer: &'a mut FrameTimer,
    name: String,
}

impl<'a> TimerGuard<'a> {
    pub fn new(timer: &'a mut FrameTimer, name: impl Into<String>) -> Self {
        let name = name.into();
        timer.mark(format!("{}_start", name));
        Self { timer, name }
    }
}

impl<'a> Drop for TimerGuard<'a> {
    fn drop(&mut self) {
        self.timer.mark(format!("{}_end", self.name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_frame_timer() {
        let mut timer = FrameTimer::new();
        
        timer.mark("logic_start");
        std::thread::sleep(Duration::from_millis(2));
        timer.mark("logic_end");
        
        timer.mark("render_start");
        std::thread::sleep(Duration::from_millis(3));
        timer.mark("render_end");
        
        let report = timer.report();
        
        let logic_time = report.get("logic");
        let render_time = report.get("render");
        
        assert!(logic_time >= 1.5 && logic_time < 5.0, "Logic time should be ~2ms");
        assert!(render_time >= 2.5 && render_time < 6.0, "Render time should be ~3ms");
        
        report.print();
    }
}
