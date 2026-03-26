// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_view::{AlignContent, AlignItems, BaseView, FlexWrap, Text, View};
use std::sync::atomic::{AtomicU32, Ordering};

#[no_mangle]
pub extern "C" fn main() {
    // 1. Root container (ID 0) - 启用 flex wrap 使子元素自动换行
    // align_content(FlexStart) 让所有行靠顶部排列，避免行间距过大
    let _root = View::new()
        .width("100%")
        .height("100%")
        .flex_wrap(FlexWrap::Wrap)
        .align_content(AlignContent::FlexStart)
        .color((10, 10, 40));

    // Create a text node
    // let text_node = Text::new()
    //     .value("Hello, Dyxel! This is a Parley text test.")
    //     .font_size(24.0)
    //     .text_color((255, 255, 255, 255))
    //     .font_family("Arial");

    // let _ = View { id: 0 }.child(text_node.id);
    for i in 1..100 {
        let child = View::new()
            .align_items(AlignItems::FlexStart)
            .width(30.0)
            .height(30.0)
            .color((200, 50, 50));

        let _ = View { id: 0 }.child(child.id);
    }

    let text_node = Text::new()
        .value("Hello, Dyxel! This is a Parley text test.")
        .font_size(24.0)
        .text_color((255, 255, 255, 255))
        .font_family("Arial");

    let _ = View { id: 0 }.child(text_node.id);

    for i in 1..100 {
        let child = View::new()
            .align_items(AlignItems::FlexStart)
            .width(30.0)
            .height(30.0)
            .color((200, 50, 50));

        let _ = View { id: 0 }.child(child.id);
    }
}

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

#[no_mangle]
pub extern "C" fn guest_tick() {
    let f = FRAME_COUNT.fetch_add(1, Ordering::SeqCst) as f32;

    // for i in 1..102 {
    //     let idx = i as f32;
    //     // Use sine and cosine functions to create smooth circular/random motion
    //     // x and y as percentages (0-100)
    //     let x = 50.0 + (f * 0.03 + idx * 0.5).cos() * 40.0;
    //     let y = 50.0 + (f * 0.02 + idx * 0.3).sin() * 40.0;

    //     let _ = View { id: i }
    //         // Correct parameter order: (top, right, bottom, left)
    //         // We set top = y, left = x, and set right/bottom to larger values to avoid interfering with layout
    //         .inset((y, 0.0, 0.0, x))
    //         // Smoother color transitions
    //         .color((
    //             (128.0 + (f * 0.02 + idx).cos() * 127.0) as u32,
    //             (128.0 + (f * 0.03 + idx * 0.5).sin() * 127.0) as u32,
    //             (128.0 + (idx * 2.0).cos() * 127.0) as u32,
    //         ));
    // }

    dyxel_view::dyxel_view_tick();
}
