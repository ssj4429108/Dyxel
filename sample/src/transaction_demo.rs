// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Transaction API 示例
//! 
//! 演示功能：
//! - 使用 Transaction 批量创建节点
//! - LayoutRegistry API 读取布局信息
//! - 文本溢出检测
//! - 瀑布流布局计算

use dyxel_view::{
    begin_transaction, AlignContent, AlignItems, BaseView, FlexWrap, Text, View,
};
use std::sync::atomic::{AtomicU32, Ordering};

#[allow(dead_code)]
/// 使用 Transaction 批量创建节点
fn _create_nodes_batch(root_id: u32, count: i32, color: (u32, u32, u32)) {
    let tx = begin_transaction();

    for _ in 0..count {
        let child = View::new()
            .align_items(AlignItems::FlexStart)
            .width(30.0)
            .height(30.0)
            .color(color);

        let _ = View { id: root_id }.child(child.id);
    }

    tx.commit();
}

pub fn init() {
    let init_tx = begin_transaction();

    // 根容器
    let _root = View::new()
        .width("100%")
        .height("100%")
        .flex_wrap(FlexWrap::Wrap)
        .align_content(AlignContent::FlexStart)
        .color((10, 10, 40));

    // 批量创建红色方块
    for _ in 1..100 {
        let child = View::new()
            .align_items(AlignItems::FlexStart)
            .width(30.0)
            .height(30.0)
            .color((200, 50, 50));

        let _ = View { id: 0 }.child(child.id);
    }

    // 文本节点
    let text_node = Text::new()
        .value("Hello, Dyxel! Transaction API Demo")
        .font_size(24.0)
        .text_color((255, 255, 255, 255))
        .font_family("Arial");

    let _ = View { id: 0 }.child(text_node.id);

    // 批量创建绿色方块
    for _ in 1..100 {
        let child = View::new()
            .align_items(AlignItems::FlexStart)
            .width(30.0)
            .height(30.0)
            .color((50, 200, 50));

        let _ = View { id: 0 }.child(child.id);
    }

    init_tx.commit();
}

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn tick() {
    let f = FRAME_COUNT.fetch_add(1, Ordering::SeqCst) as f32;

    // LayoutRegistry 示例：文本溢出检测
    if f > 10.0 {
        check_text_overflow(201);
    }

    dyxel_view::dyxel_view_tick();
}

/// LayoutRegistry 示例：文本溢出检测
fn check_text_overflow(text_node_id: u32) {
    use dyxel_view::{is_layout_dirty, take_layout};
    
    if is_layout_dirty(text_node_id) {
        let layout = take_layout(text_node_id);
        let _container_width = layout.width;
        let _container_height = layout.height;
    }
}

/// LayoutRegistry 示例：瀑布流布局计算
#[allow(dead_code)]
fn get_waterfall_position(node_id: u32, column_heights: &mut [f32]) -> (f32, f32) {
    use dyxel_view::get_layout;
    
    let shortest_col = column_heights
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0);
    
    let x = shortest_col as f32 * 100.0;
    let y = column_heights[shortest_col];
    
    let layout = get_layout(node_id);
    column_heights[shortest_col] = y + layout.height + 10.0;
    
    (x, y)
}
