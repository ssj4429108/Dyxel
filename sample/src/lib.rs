// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_view::{
    begin_transaction, with_transaction, AlignContent, AlignItems, BaseView, FlexWrap, Text,
    Transaction, View,
};
use std::sync::atomic::{AtomicU32, Ordering};

/// Batch create nodes using a transaction for better performance
fn create_nodes_batch(root_id: u32, count: i32, color: (u32, u32, u32)) {
    // Begin a transaction to batch all node creation commands
    let mut tx = begin_transaction();

    for i in 0..count {
        let child = View::new()
            .align_items(AlignItems::FlexStart)
            .width(30.0)
            .height(30.0)
            .color(color);

        let _ = View { id: root_id }.child(child.id);
    }

    // Commit the transaction - all commands are sent together
    tx.commit();
}

#[no_mangle]
pub extern "C" fn main() {
    // Use a transaction for the entire UI initialization
    let mut init_tx = begin_transaction();

    // 1. Root container (ID 0) - 启用 flex wrap 使子元素自动换行
    // align_content(FlexStart) 让所有行靠顶部排列，避免行间距过大
    let _root = View::new()
        .width("100%")
        .height("100%")
        .flex_wrap(FlexWrap::Wrap)
        .align_content(AlignContent::FlexStart)
        .color((10, 10, 40));

    // Batch create first group of children (red boxes)
    // These 99 nodes are created in a single transaction batch
    for i in 1..100 {
        let child = View::new()
            .align_items(AlignItems::FlexStart)
            .width(30.0)
            .height(30.0)
            .color((200, 50, 50));

        let _ = View { id: 0 }.child(child.id);
    }

    // // Create text node
    let text_node = Text::new()
        .value("Hello, Dyxel! Transaction API Demo")
        .font_size(24.0)
        .text_color((255, 255, 255, 255))
        .font_family("Arial");

    let _ = View { id: 0 }.child(text_node.id);

    // Batch create second group of children (green boxes)
    for i in 1..100 {
        let child = View::new()
            .align_items(AlignItems::FlexStart)
            .width(30.0)
            .height(30.0)
            .color((50, 200, 50));

        let _ = View { id: 0 }.child(child.id);
    }

    // // Commit the initialization transaction
    init_tx.commit();

    // Alternative: Using with_transaction for scoped transactions
    // dyxel_view::with_transaction(|tx| {
    //     // All operations within this closure are batched
    //     let child = View::new().width(100.0).height(100.0);
    //     View { id: 0 }.child(child.id);
    // });
}

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

#[no_mangle]
pub extern "C" fn guest_tick() {
    let f = FRAME_COUNT.fetch_add(1, Ordering::SeqCst) as f32;

    // Example: Batch update node colors using a transaction
    // Uncomment to see animated color transitions
    /*
    with_transaction(|tx| {
        for i in 1..50 {
            let idx = i as f32;
            // Smoother color transitions
            let r = (128.0 + (f * 0.02 + idx).cos() * 127.0) as u32;
            let g = (128.0 + (f * 0.03 + idx * 0.5).sin() * 127.0) as u32;
            let b = ((idx * 2.0).cos() * 127.0) as u32;

            let _ = View { id: i }.color((r, g, b));
        }
    });
    */

    dyxel_view::dyxel_view_tick();
}
