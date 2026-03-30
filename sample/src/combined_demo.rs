// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! 综合示例：Transaction API + Shadow Layout + LayoutRegistry
//! 
//! 展示三层布局系统的协同工作：
//! - Shadow Layer: 零延迟估算
//! - Registry Layer: 已提交布局
//! - Host Layer: 最终渲染

use dyxel_view::{
    init_shadow_tree, get_layout_estimated, get_layout, take_layout,
    is_layout_dirty, would_text_overflow, get_estimated_bottom_y,
    get_layouts_batch, get_layouts_range, calculate_bounds, hit_test,
    with_transaction,
    BaseView, FlexDirection, JustifyContent, AlignItems, 
    AlignContent, FlexWrap, Text, View,
};
use std::sync::atomic::{AtomicU32, Ordering};

/// 各种节点ID
static TEXT_NODE_ID: AtomicU32 = AtomicU32::new(0);
static FLEX_CONTAINER_ID: AtomicU32 = AtomicU32::new(0);
static GRID_CONTAINER_ID: AtomicU32 = AtomicU32::new(0);

pub fn init() {
    // 初始化 ShadowTree
    init_shadow_tree();
    
    dyxel_view::println("=== 综合示例：三层布局系统 ===");
    dyxel_view::println("Shadow Layer (0ms) -> Registry Layer (16ms) -> Host Layer");
    
    // 使用 Transaction 批量创建UI
    with_transaction(|_tx| {
        create_complex_layout();
    });
    
    // 立即使用 Shadow Layout 进行预计算
    perform_shadow_calculations();
}

/// 创建复杂布局结构
fn create_complex_layout() {
    // ===== 根容器 =====
    // 第一个 View::new() 创建 id=0 的节点，作为系统根节点
    let root = View::new()
        .width("100%")
        .height("100%")
        .color((15, 15, 35))
        .flex_direction(FlexDirection::Column)
        .align_items(AlignItems::Center)
        .justify_content(JustifyContent::FlexStart);
    
    // root 已经是 id=0，无需再添加到根节点
    
    // ===== Header 区域 =====
    let header = View::new()
        .width("100%")
        .height(80.0)
        .color((40, 40, 70))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    let title = Text::new()
        .value("Shadow Layout Demo")
        .font_size(28.0)
        .text_color((255, 255, 255, 255));
    
    View { id: header.id }.child(title.id);
    View { id: root.id }.child(header.id);
    
    // ===== 主要内容区域（Flex 容器）=====
    let content = View::new()
        .width("100%")
        .height(400.0)
        .color((25, 25, 45))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::SpaceAround)
        .align_items(AlignItems::Center);
    
    FLEX_CONTAINER_ID.store(content.id, Ordering::SeqCst);
    
    // 左侧卡片
    let left_card = create_card("Dynamic Text", 180.0, 250.0, (80, 120, 180));
    // 右侧卡片
    let right_card = create_card("Layout Calc", 180.0, 250.0, (120, 80, 180));
    
    View { id: content.id }.child(left_card.id);
    View { id: content.id }.child(right_card.id);
    View { id: root.id }.child(content.id);
    
    // ===== Grid 区域 =====
    let grid = create_color_grid();
    GRID_CONTAINER_ID.store(grid.id, Ordering::SeqCst);
    View { id: root.id }.child(grid.id);
    
    // ===== 状态文本 =====
    // 将文本放在固定宽度的容器中，防止换行
    let status_container = View::new()
        .width(300.0)
        .height(30.0)
        .color((15, 15, 35))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    let status_text = Text::new()
        .value("Waiting for layout...")
        .font_size(14.0)
        .text_color((200, 200, 200, 255));
    
    View { id: status_container.id }.child(status_text.id);
    TEXT_NODE_ID.store(status_text.id, Ordering::SeqCst);
    View { id: root.id }.child(status_container.id);
}

/// 创建卡片
fn create_card(title: &str, width: f32, height: f32, color: (u32, u32, u32)) -> View {
    let card = View::new()
        .width(width)
        .height(height)
        .color(color)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::FlexStart)
        .align_items(AlignItems::Center);
    
    let label = Text::new()
        .value(title)
        .font_size(16.0)
        .text_color((255, 255, 255, 255));
    
    View { id: card.id }.child(label.id);
    
    card
}

/// 创建彩色网格
fn create_color_grid() -> View {
    let grid = View::new()
        .width("100%")
        .height(200.0)
        .color((20, 20, 40))
        .flex_wrap(FlexWrap::Wrap)
        .justify_content(JustifyContent::Center)
        .align_content(AlignContent::Center);
    
    let colors = [
        (255, 100, 100), (100, 255, 100), (100, 100, 255),
        (255, 255, 100), (255, 100, 255), (100, 255, 255),
        (200, 150, 100), (150, 100, 200), (100, 200, 150),
        (200, 100, 150), (150, 200, 100), (100, 150, 200),
    ];
    
    for &color in &colors {
        let cell = View::new()
            .width(60.0)
            .height(60.0)
            .color(color);
        View { id: grid.id }.child(cell.id);
    }
    
    grid
}

/// 使用 Shadow Layout 进行预计算
fn perform_shadow_calculations() {
    dyxel_view::println("\n[Shadow Layer] 零延迟预计算:");
    
    // 1. 预估 Flex 容器布局
    let flex_id = FLEX_CONTAINER_ID.load(Ordering::SeqCst);
    if flex_id > 0 {
        let layout = get_layout_estimated(flex_id);
        dyxel_view::println(&format!(
            "  Flex容器: w={:.1}, h={:.1}",
            layout.width, layout.height
        ));
        
        // 预估子元素位置
        for i in 0..2 {
            let child_id = flex_id + 1 + i;
            let child_layout = get_layout_estimated(child_id);
            dyxel_view::println(&format!(
                "    子元素{}: x={:.1}, y={:.1}",
                i, child_layout.x, child_layout.y
            ));
        }
    }
    
    // 2. 预估网格总高度
    let grid_id = GRID_CONTAINER_ID.load(Ordering::SeqCst);
    if grid_id > 0 {
        if let Some(bottom) = get_estimated_bottom_y(grid_id) {
            dyxel_view::println(&format!(
                "  Grid容器预估底部Y: {:.1}px", bottom
            ));
        }
    }
}

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn tick() {
    let frame = FRAME_COUNT.fetch_add(1, Ordering::SeqCst);
    
    // ===== 第10帧：对比 Shadow vs Registry =====
    if frame == 10 {
        compare_shadow_vs_registry();
    }
    
    // ===== 第20帧：文本溢出检测 =====
    if frame == 20 {
        check_text_overflow_demo();
    }
    
    // ===== 第30帧：批量API演示 =====
    if frame == 30 {
        batch_api_demo();
    }
    
    // ===== 每60帧报告完整状态 =====
    if frame % 60 == 0 && frame > 0 {
        report_full_status(frame);
    }
    
    dyxel_view::dyxel_view_tick();
}

/// 对比 Shadow Layout 和 LayoutRegistry
fn compare_shadow_vs_registry() {
    dyxel_view::println("\n[对比] Shadow vs Registry:");
    
    let grid_id = GRID_CONTAINER_ID.load(Ordering::SeqCst);
    if grid_id == 0 { return; }
    
    // Shadow Layer（零延迟）
    let shadow_layout = get_layout_estimated(grid_id);
    dyxel_view::println(&format!(
        "  [Shadow]  x={:.1}, y={:.1}, w={:.1}, h={:.1}",
        shadow_layout.x, shadow_layout.y,
        shadow_layout.width, shadow_layout.height
    ));
    
    // Registry Layer（可能有延迟）
    let registry_layout = get_layout(grid_id);
    dyxel_view::println(&format!(
        "  [Registry] x={:.1}, y={:.1}, w={:.1}, h={:.1}",
        registry_layout.x, registry_layout.y,
        registry_layout.width, registry_layout.height
    ));
    
    // 计算差异
    let diff_x = (shadow_layout.x - registry_layout.x).abs();
    let diff_y = (shadow_layout.y - registry_layout.y).abs();
    if diff_x > 0.1 || diff_y > 0.1 {
        dyxel_view::println(&format!(
            "  ⚠️ 位置差异: dx={:.1}, dy={:.1}", diff_x, diff_y
        ));
    } else {
        dyxel_view::println("  ✓ Shadow 与 Registry 一致");
    }
}

/// 文本溢出检测演示
fn check_text_overflow_demo() {
    dyxel_view::println("\n[文本溢出检测]:");
    
    let text_id = TEXT_NODE_ID.load(Ordering::SeqCst);
    if text_id == 0 { return; }
    
    // 模拟不同长度的文本
    let test_cases = [
        ("Short text", 50.0),
        ("Medium length text", 150.0),
        ("Very long text content", 300.0),
    ];
    
    for (desc, content_width) in &test_cases {
        let would_overflow = would_text_overflow(text_id, *content_width);
        let status = if would_overflow { "❌ 溢出" } else { "✓ 适配" };
        dyxel_view::println(&format!(
            "  {} (需要{:.0}px): {}", desc, content_width, status
        ));
    }
}

/// 报告完整状态
fn report_full_status(frame: u32) {
    dyxel_view::println(&format!("\n=== 第{}帧状态报告 ===", frame));
    
    // Flex 容器状态
    let flex_id = FLEX_CONTAINER_ID.load(Ordering::SeqCst);
    if flex_id > 0 {
        let layout = get_layout_estimated(flex_id);
        dyxel_view::println(&format!(
            "Flex容器: w={:.1}, h={:.1} (预估)",
            layout.width, layout.height
        ));
    }
    
    // Grid 容器状态
    let grid_id = GRID_CONTAINER_ID.load(Ordering::SeqCst);
    if grid_id > 0 {
        if let Some(bottom) = get_estimated_bottom_y(grid_id) {
            dyxel_view::println(&format!(
                "Grid容器底部: {:.1}px (预估)", bottom
            ));
        }
    }
    
    // 检查是否有新的 Registry 布局
    let text_id = TEXT_NODE_ID.load(Ordering::SeqCst);
    if text_id > 0 && is_layout_dirty(text_id) {
        let layout = take_layout(text_id);
        dyxel_view::println(&format!(
            "文本节点新布局: x={:.1}, y={:.1}",
            layout.x, layout.y
        ));
    }
}

/// 批量API演示
fn batch_api_demo() {
    dyxel_view::println("\n[批量API演示]:");
    
    let grid_id = GRID_CONTAINER_ID.load(Ordering::SeqCst);
    if grid_id == 0 { return; }
    
    // 1. 批量查询前6个格子的布局（使用Range）
    let start_id = grid_id + 1; // 跳过容器本身
    let end_id = start_id + 6;
    let layouts = get_layouts_range(start_id, end_id);
    
    dyxel_view::println(&format!("  批量查询 {} 个格子布局:", layouts.len()));
    for (i, layout) in layouts.iter().enumerate() {
        dyxel_view::println(&format!(
            "    格子{}: x={:.0}, y={:.0}, w={:.0}, h={:.0}",
            i, layout.x, layout.y, layout.width, layout.height
        ));
    }
    
    // 2. 批量查询指定ID（使用数组）
    let specific_ids = [start_id, start_id + 3, start_id + 5];
    let specific_layouts = get_layouts_batch(&specific_ids);
    
    dyxel_view::println("  指定ID批量查询:");
    for (i, layout) in specific_layouts.iter().enumerate() {
        dyxel_view::println(&format!(
            "    ID {}: x={:.0}, y={:.0}",
            specific_ids[i], layout.x, layout.y
        ));
    }
    
    // 3. 计算边界框
    let all_ids: Vec<u32> = (start_id..end_id).collect();
    if let Some((x, y, w, h)) = calculate_bounds(&all_ids) {
        dyxel_view::println(&format!(
            "  边界框: x={:.0}, y={:.0}, w={:.0}, h={:.0}",
            x, y, w, h
        ));
    }
    
    // 4. 点击测试演示
    let test_point = (100.0, 150.0);
    let hit_count = all_ids.iter()
        .filter(|&&id| hit_test(id, test_point.0, test_point.1))
        .count();
    dyxel_view::println(&format!(
        "  点击测试 ({}, {}): {} 个节点被命中",
        test_point.0, test_point.1, hit_count
    ));
}
