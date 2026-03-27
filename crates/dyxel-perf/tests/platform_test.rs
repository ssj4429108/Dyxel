// Integration test for platform-specific system info

use dyxel_perf::{PerformanceMonitor, PerfConfig};
use std::thread;
use std::time::Duration;

#[test]
fn test_macos_memory() {
    let monitor = PerformanceMonitor::new(PerfConfig {
        enable_fps: true,
        enable_cpu: true,
        enable_memory: true,
        enable_overlay: false,
        fps_sample_count: 60,
        system_sample_interval_ms: 100, // Short interval for testing
        overlay_position: (10.0, 10.0),
        overlay_scale: 1.0,
    });
    
    println!("Platform: {}", monitor.platform_name());
    
    // Simulate some frames
    for i in 0..5 {
        thread::sleep(Duration::from_millis(16));
        monitor.begin_frame();
        
        // Force system stats update on first frame
        if i == 0 {
            thread::sleep(Duration::from_millis(150));
        }
        
        let stats = monitor.get_stats();
        println!(
            "Frame {}: FPS={:.1}, Time={:.2}ms, CPU={:.1}%, Mem={:.1}MB, Total={}",
            i + 1,
            stats.fps,
            stats.frame_time_ms,
            stats.cpu_usage,
            stats.memory_used_mb,
            stats.total_frames
        );
    }
    
    // Assertions - these should pass on macOS
    let final_stats = monitor.get_stats();
    assert!(final_stats.total_frames >= 5, "Should have tracked at least 5 frames");
    
    // Memory should be non-zero on macOS
    if monitor.platform_name() == "macos" {
        assert!(final_stats.memory_used_mb > 0.0, "Memory usage should be > 0 on macOS");
        // CPU might be 0 on first sample, but should eventually be non-zero
        println!("Memory: {:.2} MB", final_stats.memory_used_mb);
    }
    
    println!("✅ Platform test passed!");
}

#[test]
fn test_overlay_toggle() {
    let mut monitor = PerformanceMonitor::with_overlay();
    
    assert!(monitor.should_show_overlay(), "Overlay should be enabled");
    
    monitor.toggle_overlay();
    assert!(!monitor.should_show_overlay(), "Overlay should be disabled after toggle");
    
    monitor.toggle_overlay();
    assert!(monitor.should_show_overlay(), "Overlay should be enabled after second toggle");
    
    let (x, y, scale) = monitor.get_overlay_config();
    assert_eq!(x, 10.0);
    assert_eq!(y, 10.0);
    assert_eq!(scale, 1.0);
    
    println!("✅ Overlay toggle test passed!");
}
