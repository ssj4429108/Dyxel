// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shadow Layout 示例 - 修复版
//!
//! 本示例演示 Shadow Layout 的核心功能：
//! - 零延迟布局查询
//! - 瀑布流布局预计算（真正的瀑布流效果）
//! - 响应式布局估算

use dyxel_view::{
    get_estimated_bottom_y, get_layout_estimated, init_shadow_tree, set_viewport_size,
    AlignContent, AlignItems, BaseView, FlexDirection, FlexWrap, JustifyContent, Text, View,
};
use std::sync::atomic::{AtomicU32, Ordering};

/// 各种节点ID存储
static WATERFALL_CONTAINER_ID: AtomicU32 = AtomicU32::new(0);
static WATERFALL_ITEM_IDS: AtomicU32 = AtomicU32::new(0); // 存储第一个item的ID

pub fn init() {
    // 初始化 ShadowTree
    init_shadow_tree();

    // 设置视口大小
    // set_viewport_size(390.0, 844.0);

    // 创建根容器 - 使用 Column 布局使示例垂直排列
    let _root = View::new()
        .width("100%")
        .height("100%")
        .color((15, 15, 30))
        .flex_direction(FlexDirection::Column)
        .align_items(AlignItems::Center)
        .justify_content(JustifyContent::FlexStart)
        .padding((20.0, 20.0, 20.0, 20.0));

    // ===== 示例1：瀑布流布局（真正的瀑布流效果）=====
    create_real_waterfall_demo();

    // ===== 示例2：布局查询演示 =====
    create_layout_query_demo();
}

/// 真正的瀑布流布局实现
///
/// 使用 Flex Wrap + 固定列宽实现瀑布流效果
fn create_real_waterfall_demo() {
    // 标题
    let title = Text::new()
        .value("Masonry Layout Demo")
        .font_size(18.0)
        .text_color((255, 255, 255, 255));
    View { id: 0 }.child(title.id);

    // 瀑布流容器 - 使用 Wrap 实现多行
    let container = View::new()
        .width("100%")
        .height(400.0) // 固定高度，内容会换行
        .color((25, 25, 45))
        .flex_direction(FlexDirection::Row)
        .flex_wrap(FlexWrap::Wrap)
        .align_content(AlignContent::FlexStart)
        .justify_content(JustifyContent::SpaceBetween);

    View { id: 0 }.child(container.id);
    WATERFALL_CONTAINER_ID.store(container.id, Ordering::SeqCst);

    // 创建不同高度的项目（模拟图片/卡片）
    let items_data = [
        (120.0, (255, 100, 100), "Item 1"),
        (180.0, (100, 255, 100), "Item 2"),
        (150.0, (100, 100, 255), "Item 3"),
        (200.0, (255, 255, 100), "Item 4"),
        (100.0, (255, 100, 255), "Item 5"),
        (160.0, (100, 255, 255), "Item 6"),
        (140.0, (200, 150, 100), "Item 7"),
        (190.0, (150, 100, 200), "Item 8"),
    ];

    // 存储第一个item的ID
    let first_item_id = View::new().id;
    WATERFALL_ITEM_IDS.store(first_item_id, Ordering::SeqCst);

    for (_i, (height, color, label)) in items_data.iter().enumerate() {
        // 每个项目容器 - 固定宽度，不同高度
        let item = View::new()
            .width(48.0) // 约50% 减去间距
            .height(*height)
            .color(*color)
            .flex_direction(FlexDirection::Column)
            .justify_content(JustifyContent::Center)
            .align_items(AlignItems::Center);

        // 项目标签
        let item_text = Text::new()
            .value(*label)
            .font_size(12.0)
            .text_color((255, 255, 255, 255));
        View { id: item.id }.child(item_text.id);

        View { id: container.id }.child(item.id);

        // 使用 Shadow Layout 查询预估布局
        let layout = get_layout_estimated(item.id);
        dyxel_view::println(&format!(
            "[Shadow] {} 预估: x={:.1}, y={:.1}, w={:.1}, h={:.1}",
            label, layout.x, layout.y, layout.width, layout.height
        ));
    }

    // 预测容器总高度
    if let Some(bottom) = get_estimated_bottom_y(container.id) {
        dyxel_view::println(&format!("[Shadow] 瀑布流容器预估底部: {:.1}px", bottom));
    }
}

/// 布局查询演示 - 展示 Flex SpaceBetween 效果
fn create_layout_query_demo() {
    // 间距
    let spacer = View::new().width("100%").height(20.0).color((15, 15, 30)); // 透明效果
    View { id: 0 }.child(spacer.id);

    // 标题
    let title = Text::new()
        .value("Flex SpaceBetween Layout")
        .font_size(18.0)
        .text_color((255, 255, 255, 255));
    View { id: 0 }.child(title.id);

    // 响应式容器 - 使用 SpaceBetween 分布子元素
    let responsive = View::new()
        .width("100%")
        .height(80.0)
        .color((40, 40, 60))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::SpaceBetween)
        .align_items(AlignItems::Center)
        .padding((10.0, 0.0, 10.0, 0.0));

    View { id: 0 }.child(responsive.id);

    // 添加三个子视图 - 应该有明显间距
    let child_configs = [
        ("Left", (100, 150, 200)),
        ("Mid", (150, 100, 200)),
        ("Right", (200, 100, 150)),
    ];

    for (label, color) in &child_configs {
        let child = View::new()
            .width(60.0)
            .height(50.0)
            .color(*color)
            .flex_direction(FlexDirection::Column)
            .justify_content(JustifyContent::Center)
            .align_items(AlignItems::Center);

        let text = Text::new()
            .value(*label)
            .font_size(10.0)
            .text_color((255, 255, 255, 255));
        View { id: child.id }.child(text.id);

        View { id: responsive.id }.child(child.id);
    }

    // 查询容器布局
    let container_layout = get_layout_estimated(responsive.id);
    dyxel_view::println(&format!(
        "[Shadow] 响应式容器: w={:.1}, h={:.1}",
        container_layout.width, container_layout.height
    ));
}

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn tick() {
    let frame = FRAME_COUNT.fetch_add(1, Ordering::SeqCst);

    // 每60帧报告一次瀑布流状态
    if frame % 60 == 0 && frame > 0 {
        report_waterfall_status(frame);
    }

    dyxel_view::dyxel_view_tick();
}

/// 报告瀑布流布局状态
fn report_waterfall_status(frame: u32) {
    let container_id = WATERFALL_CONTAINER_ID.load(Ordering::SeqCst);
    if container_id == 0 {
        return;
    }

    dyxel_view::println(&format!("\n=== 第{}帧 瀑布流状态 ===", frame));

    // 容器布局
    let layout = get_layout_estimated(container_id);
    dyxel_view::println(&format!(
        "容器: x={:.1}, y={:.1}, w={:.1}, h={:.1}",
        layout.x, layout.y, layout.width, layout.height
    ));

    // 获取底部位置
    if let Some(bottom) = get_estimated_bottom_y(container_id) {
        dyxel_view::println(&format!("预估底部 Y: {:.1}px", bottom));
    }
}
