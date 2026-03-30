// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dual-Track 1000 Nodes Stress Test
//!
//! Uses Dual-Track memory architecture:
//! - Registry (32KB): Static node storage
//! - CommandStream (96KB): Dynamic property updates

use dyxel_view::{
    BaseView, FlexDirection, FlexWrap, AlignContent,
    View, Text,
};

// println for WASM
#[cfg(target_arch = "wasm32")]
fn println(s: &str) {
    // WASM: no-op or host call
}

#[cfg(not(target_arch = "wasm32"))]
fn println(s: &str) {
    std::println!("{}", s);
}
use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};

/// Animation frame counter
static FRAME: AtomicU32 = AtomicU32::new(0);

/// Node start ID (first animated node)
static NODES_START: AtomicU32 = AtomicU32::new(0);

/// Root container ID
static ROOT_ID: AtomicU32 = AtomicU32::new(0);

/// Initialization complete flag
static INIT_COMPLETE: AtomicBool = AtomicBool::new(false);

/// Current page during init
static CURRENT_PAGE: AtomicU32 = AtomicU32::new(0);

/// Total nodes to create
const TOTAL_NODES: usize = 1000;

/// Nodes per page
const PAGE_SIZE: usize = 200;

pub fn init() {
    println("=== Dual-Track 1000 Nodes Test ===");
    println("Memory: Registry(32KB) + CommandStream(96KB)");
    
    // Create root container
    let root = View::new()
        .width("100%")
        .height("100%")
        .color((10, 10, 20))
        .flex_direction(FlexDirection::Row)
        .flex_wrap(FlexWrap::Wrap)
        .align_content(AlignContent::FlexStart);
    
    ROOT_ID.store(root.id, Ordering::SeqCst);
    NODES_START.store(root.id + 1, Ordering::SeqCst);
    
    println(&format!("Root ID: {}, Nodes start at: {}", root.id, root.id + 1));
    
    // Note: Actual Dual-Track integration would use:
    // - Registry for node structure
    // - CommandStream for properties
    // For now, we use traditional API with manual paging
    
    // Start creating nodes in pages
    // Page 0 will be created in init(), pages 1-4 in tick()
    create_page(0, root.id);
    
    println("Init page 0/4 complete. Continue in tick()...");
}

/// Create one page of nodes (200 nodes)
fn create_page(page: u32, root_id: u32) {
    let start_idx = page as usize * PAGE_SIZE;
    let end_idx = ((page as usize + 1) * PAGE_SIZE).min(TOTAL_NODES);
    
    let cols = 25;
    let cell_width = 800.0 / cols as f32;
    let cell_height = 600.0 / (TOTAL_NODES / cols) as f32;
    
    for i in start_idx..end_idx {
        // Initial color based on position (gradient)
        let hue = (i as f32 / TOTAL_NODES as f32 * 360.0) as u32;
        let color = hsv_to_rgb(hue, 0.8, 0.9);
        
        let node = View::new()
            .width(cell_width - 2.0)
            .height(cell_height - 2.0)
            .color(color);
        
        View { id: root_id }.child(node.id);
    }
    
    CURRENT_PAGE.store(page + 1, Ordering::SeqCst);
    
    println(&format!(
        "Page {}/4: Created nodes {}-{}",
        page + 1,
        start_idx,
        end_idx - 1
    ));
}

/// HSV to RGB conversion
fn hsv_to_rgb(h: u32, s: f32, v: f32) -> (u32, u32, u32) {
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
        ((r + m) * 255.0) as u32,
        ((g + m) * 255.0) as u32,
        ((b + m) * 255.0) as u32,
    )
}

/// Update node colors (rainbow wave)
fn update_colors(frame: u32) {
    let nodes_start = NODES_START.load(Ordering::SeqCst);
    if nodes_start == 0 {
        return;
    }
    
    let time = frame as f32 * 0.03;
    
    // Adaptive stride - update fewer nodes under pressure
    // In real Dual-Track, this would check CommandStream usage
    let stride = if frame % 10 == 0 { 1 } else { 3 }; // Simplified backpressure
    
    for i in (0..TOTAL_NODES).step_by(stride) {
        let node_id = nodes_start + i as u32;
        
        // Rainbow wave
        let phase = i as f32 * 0.05;
        let hue = ((time * 50.0 + phase * 100.0) % 360.0) as u32;
        let saturation = 0.7 + (time + phase).sin() * 0.2;
        let value = 0.8 + (time * 0.5 + phase).cos() * 0.15;
        
        let color = hsv_to_rgb(
            hue,
            saturation.clamp(0.0, 1.0),
            value.clamp(0.0, 1.0)
        );
        
        View { id: node_id }.color(color);
    }
}

/// Report statistics
fn report_stats(frame: u32) {
    let nodes_start = NODES_START.load(Ordering::SeqCst);
    let init = INIT_COMPLETE.load(Ordering::SeqCst);
    
    println(&format!(
        "[Frame {}] Nodes: {} | Init: {} | Backpressure: N/A",
        frame,
        if nodes_start > 0 { "1000" } else { "0" },
        if init { "Complete" } else { "In Progress" }
    ));
}

pub fn tick() {
    let frame = FRAME.fetch_add(1, Ordering::SeqCst);
    let root_id = ROOT_ID.load(Ordering::SeqCst);
    let current_page = CURRENT_PAGE.load(Ordering::SeqCst);
    
    // Continue initialization if needed
    if !INIT_COMPLETE.load(Ordering::SeqCst) {
        if current_page < 5 {
            // Create next page every 10 frames
            if frame % 10 == 0 {
                create_page(current_page, root_id);
                
                // Flush commands after each page
                dyxel_view::dyxel_view_tick();
                return; // Skip animation this frame
            }
        } else {
            INIT_COMPLETE.store(true, Ordering::SeqCst);
            println("=== Initialization Complete ===");
            
            // Add status text
            let status = Text::new()
                .value("1000 nodes - Rainbow Wave (Dual-Track Demo)")
                .font_size(14.0)
                .text_color((255, 255, 255, 255));
            View { id: root_id }.child(status.id);
        }
    }
    
    // Animation phase
    if INIT_COMPLETE.load(Ordering::SeqCst) {
        update_colors(frame);
        
        // Report every 60 frames
        if frame % 60 == 0 {
            report_stats(frame);
        }
    }
    
    dyxel_view::dyxel_view_tick();
}
