// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dual-Track WASM API
//!
//! WASM-side interface for Dual-Track memory architecture.
//! Provides initialization paging, backpressure handling, and command batching.

use dyxel_shared::dual_track::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// Current page during initialization
static INIT_PAGE: AtomicU32 = AtomicU32::new(0);

/// Total pages needed
static INIT_TOTAL_PAGES: AtomicU32 = AtomicU32::new(0);

/// Page size (nodes per page)
pub const PAGE_SIZE: usize = 200;

/// Timeout for reserve_space (milliseconds)
pub const RESERVE_TIMEOUT_MS: u32 = 16;

/// Result type for dual-track operations
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DualTrackResult {
    Ok,
    RegistryFull,
    CommandStreamFull,
    Timeout,
    TicketMismatch,
    SentinelCorrupted,
}

/// Initialize dual-track memory from WASM side
/// 
/// # Safety
/// Must be called before any other dual-track operations
pub unsafe fn init_dual_track(registry: *mut Registry, stream: *mut CommandStream) {
    (*registry).initialize();
    (*stream).initialize();
}

/// Bulk create nodes with paging
/// 
/// This function implements "Initialization Paging" - splitting large
/// node creation into smaller chunks to avoid buffer overflow.
/// 
/// # Example
/// ```rust,ignore
/// // Create 1000 nodes in 5 pages of 200
/// for page in 0..5 {
///     let start_id = page * 200;
///     let count = create_node_page(registry, start_id, 200, 0);
///     if count < 200 {
///         // Handle partial failure
///     }
///     
///     // Signal host to process this page
///     signal_host_flush();
/// }
/// ```
pub fn create_node_page(
    registry: &Registry,
    start_id: u32,
    count: usize,
    parent_id: u32,
) -> usize {
    let mut created = 0;
    
    for i in 0..count {
        let node_id = start_id + i as u32;
        
        let node = RegistryNode {
            id: node_id,
            parent_id,
            node_type: NodeType::Container as u8,
            init_mask: 0, // Properties set separately via CommandStream
            flags: 0,
            style_idx: 0,
            _reserved: 0,
        };
        
        unsafe {
            if registry.add_node(node).is_none() {
                break; // Registry full
            }
        }
        
        created += 1;
    }
    
    created
}

/// Reserve space in command stream with backpressure handling
/// 
/// This function implements "Explicit Backpressure" - if the buffer
/// is nearly full, it will wait (spin) for host to consume data.
/// 
/// # Returns
/// - `Ok(position)` if space reserved successfully
/// - `Err(DualTrackResult)` if timeout or stream full
pub fn reserve_space(
    stream: &CommandStream,
    size: usize,
) -> Result<usize, DualTrackResult> {
    // Fast path: check if space available without waiting
    if stream.free_space() as usize >= size + SAFETY_MARGIN {
        return unsafe {
            stream.reserve_space(size)
                .ok_or(DualTrackResult::CommandStreamFull)
        };
    }
    
    // Slow path: wait for host with timeout
    let start_time = get_time_ms();
    
    loop {
        // Try to reserve
        if let Some(pos) = unsafe { stream.reserve_space(size) } {
            return Ok(pos);
        }
        
        // Check timeout
        if get_time_ms() - start_time > RESERVE_TIMEOUT_MS {
            return Err(DualTrackResult::Timeout);
        }
        
        // Yield to host (in real WASM, this might be a host call)
        spin_yield();
    }
}

/// Write a set color command
/// 
/// # Example
/// ```rust,ignore
/// let pos = reserve_space(stream, 5)?;
/// write_set_color(stream, node_id, 255, 0, 0, pos);
/// ```
pub fn write_set_color(
    stream: &CommandStream,
    node_id: u32,
    r: u8,
    g: u8,
    b: u8,
    pos: usize,
) -> bool {
    let data = [
        (node_id & 0xFF) as u8,
        ((node_id >> 8) & 0xFF) as u8,
        r,
        g,
        b,
    ];
    
    unsafe { stream.write_command(OP_SET_COLOR, &data, pos) }
}

/// Write a bulk create command (compact node creation)
pub fn write_bulk_create(
    stream: &CommandStream,
    start_id: u32,
    count: u16,
    parent_id: u32,
    pos: usize,
) -> bool {
    let data = [
        (start_id & 0xFF) as u8,
        ((start_id >> 8) & 0xFF) as u8,
        (count & 0xFF) as u8,
        ((count >> 8) & 0xFF) as u8,
        (parent_id & 0xFF) as u8,
        ((parent_id >> 8) & 0xFF) as u8,
    ];
    
    unsafe { stream.write_command(OP_BULK_CREATE, &data, pos) }
}

/// Commit all pending commands
/// 
/// This increments the ticket, signaling host that new data is ready
pub fn commit_commands(stream: &CommandStream) {
    stream.commit();
}

/// Check current backpressure level
pub fn check_backpressure(stream: &CommandStream) -> ThrottleLevel {
    stream.throttle_level()
}

/// Get current command stream usage percentage
pub fn get_usage_percent(stream: &CommandStream) -> f32 {
    stream.usage_percent()
}

/// Validate entire dual-track memory
pub fn validate_dual_track(registry: &Registry, stream: &CommandStream) -> DualTrackResult {
    if !registry.check_sentinel() || !stream.check_sentinel() {
        return DualTrackResult::SentinelCorrupted;
    }
    
    if !registry.is_valid() || !stream.is_valid() {
        return DualTrackResult::SentinelCorrupted;
    }
    
    DualTrackResult::Ok
}

/// Initialize 1000 nodes using paging
/// 
/// This is the main entry point for stress test initialization
pub fn init_1000_nodes_paged(
    registry: &Registry,
    stream: &CommandStream,
    root_id: u32,
) -> Result<(), DualTrackResult> {
    const TOTAL_NODES: usize = 1000;
    const PAGES: usize = (TOTAL_NODES + PAGE_SIZE - 1) / PAGE_SIZE;
    
    INIT_TOTAL_PAGES.store(PAGES as u32, Ordering::Relaxed);
    
    for page in 0..PAGES {
        let start_id = root_id + 1 + (page * PAGE_SIZE) as u32;
        let remaining = TOTAL_NODES - (page * PAGE_SIZE);
        let count = remaining.min(PAGE_SIZE);
        
        // Create nodes in registry
        let created = create_node_page(registry, start_id, count, root_id);
        if created < count {
            return Err(DualTrackResult::RegistryFull);
        }
        
        // Also send bulk create command for Host compatibility
        let cmd_size = 7;
        let pos = reserve_space(stream, cmd_size)?;
        write_bulk_create(stream, start_id, created as u16, root_id, pos);
        commit_commands(stream);
        
        // Update page counter
        INIT_PAGE.store(page as u32 + 1, Ordering::Relaxed);
        
        // Signal host to process (in real implementation, this would trigger render)
        // For now, we just commit and continue
    }
    
    Ok(())
}

/// Get initialization progress (0-100)
pub fn get_init_progress() -> u32 {
    let current = INIT_PAGE.load(Ordering::Relaxed);
    let total = INIT_TOTAL_PAGES.load(Ordering::Relaxed);
    
    if total == 0 {
        0
    } else {
        (current * 100 / total).min(100)
    }
}

/// Update 1000 node colors with backpressure awareness
/// 
/// This implements adaptive update rate based on buffer pressure
pub fn update_1000_colors(
    stream: &CommandStream,
    start_id: u32,
    frame: u32,
) -> Result<u32, DualTrackResult> {
    // Determine update stride based on backpressure
    let stride = match check_backpressure(stream) {
        ThrottleLevel::Normal => 1,      // Update all
        ThrottleLevel::Elevated => 2,    // Update 1/2
        ThrottleLevel::Warning => 5,     // Update 1/5
        ThrottleLevel::Critical => 10,   // Update 1/10
    };
    
    let time = frame as f32 * 0.05;
    let mut updated = 0;
    
    for i in (0..1000).step_by(stride) {
        let node_id = start_id + i as u32;
        
        // Calculate rainbow color
        let phase = i as f32 * 0.05;
        let hue = ((time * 30.0 + phase * 100.0) % 360.0) as u32;
        let (r, g, b) = hsv_to_rgb(hue, 0.8, 0.9);
        
        // Reserve space and write command
        let pos = reserve_space(stream, 5)?;
        write_set_color(stream, node_id, r, g, b, pos);
        updated += 1;
    }
    
    commit_commands(stream);
    Ok(updated)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get current time in milliseconds (WASM stub)
#[cfg(target_arch = "wasm32")]
fn get_time_ms() -> u32 {
    // In real WASM, this would call host function
    // For now, return frame count as approximation
    0
}

#[cfg(not(target_arch = "wasm32"))]
fn get_time_ms() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u32
}

/// Yield to host (WASM stub)
#[cfg(target_arch = "wasm32")]
fn spin_yield() {
    // In real WASM, this might call host yield function
    std::hint::spin_loop();
}

#[cfg(not(target_arch = "wasm32"))]
fn spin_yield() {
    std::thread::yield_now();
}

/// HSV to RGB conversion
fn hsv_to_rgb(h: u32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h % 360;
    let c = v * s;
    let x = c * (1.0 - ((h as f32 / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    
    let (r, g, b) = match h / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Signal host to flush (placeholder)
/// 
/// In real implementation, this would call host function
pub fn signal_host_flush() {
    // Placeholder - actual implementation depends on host interface
}

// ============================================================================
// Integration with existing dyxel-view
// ============================================================================

/// Initialize dual-track and create 1000 nodes
pub fn init_stress_test_dual_track() -> Result<(), &'static str> {
    // In real implementation, get pointers from shared memory
    // For now, this is a placeholder
    
    // unsafe {
    //     let registry = get_registry_ptr();
    //     let stream = get_command_stream_ptr();
    //     init_dual_track(registry, stream);
    //     init_1000_nodes_paged(&*registry, &*stream, 0)?;
    // }
    
    Ok(())
}

/// Tick function for stress test (adaptive update)
pub fn tick_stress_test(_frame: u32) -> Result<u32, &'static str> {
    // Placeholder - would use actual dual-track memory
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_hsv_to_rgb() {
        let (r, g, b) = hsv_to_rgb(0, 1.0, 1.0); // Red
        assert!(r > 250);
        assert!(g < 10);
        assert!(b < 10);
        
        let (r, g, b) = hsv_to_rgb(120, 1.0, 1.0); // Green
        assert!(r < 10);
        assert!(g > 250);
        assert!(b < 10);
        
        let (r, g, b) = hsv_to_rgb(240, 1.0, 1.0); // Blue
        assert!(r < 10);
        assert!(g < 10);
        assert!(b > 250);
    }
    
    #[test]
    fn test_backpressure_levels() {
        // This would need actual CommandStream in test
        // For now, just verify enum values
        assert!(ThrottleLevel::Normal < ThrottleLevel::Elevated);
        assert!(ThrottleLevel::Elevated < ThrottleLevel::Warning);
        assert!(ThrottleLevel::Warning < ThrottleLevel::Critical);
    }
}
