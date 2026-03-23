// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::atomic::{AtomicU32, Ordering};
use dyxel_view::{View, BaseView, PositionType};

#[no_mangle]
pub extern "C" fn main() {
    // 1. Root container (ID 0)
    let _root = View::new()
        .width("100%")
        .height("100%")
        .color((10, 10, 40)); 
    
    // 2. Create 100 dynamic blocks and mount them
    for _ in 1..101 {
        let child = View::new()
            .position(PositionType::Absolute)
            .width(30.0)
            .height(30.0);
        
        let _ = View { id: 0 }.child(child.id);
    }
}

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

#[no_mangle]
pub extern "C" fn guest_tick() {
    let f = FRAME_COUNT.fetch_add(1, Ordering::SeqCst) as f32;
    
    for i in 1..101 {
        let idx = i as f32;
        // Use sine and cosine functions to create smooth circular/random motion
        // x and y as percentages (0-100)
        let x = 50.0 + (f * 0.03 + idx * 0.5).cos() * 40.0; 
        let y = 50.0 + (f * 0.02 + idx * 0.3).sin() * 40.0; 
        
        let _ = View { id: i }
            // Correct parameter order: (top, right, bottom, left)
            // We set top = y, left = x, and set right/bottom to larger values to avoid interfering with layout
            .inset((y, 0.0, 0.0, x)) 
            // Smoother color transitions
            .color((
                (128.0 + (f * 0.02 + idx).cos() * 127.0) as u32,
                (128.0 + (f * 0.03 + idx * 0.5).sin() * 127.0) as u32,
                (128.0 + (idx * 2.0).cos() * 127.0) as u32
            ));
    }
    
    dyxel_view::dyxel_view_tick();
}
