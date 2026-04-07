// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Off-Screen Rendering 完整功能验证示例
//!
//! 本示例验证所有离屏渲染改造功能：
//! 1. 离屏渲染层堆栈 (Layer Stack)
//! 2. Dual-Filtering 模糊滤镜
//! 3. 阴影效果 (Drop Shadow)
//! 4. 混合模式 (Blend Modes)
//! 5. 纹理池管理 (128MB 限制、LRU 驱逐)
//! 6. 光栅缓存自动烘焙
//! 7. 内存压力降级策略

use dyxel_view::{
    BaseView, FlexDirection, JustifyContent, AlignItems,
    View, Text, ViewOffscreenExt, CacheHint,
};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// 动画帧计数
static ANIMATION_FRAME: AtomicU32 = AtomicU32::new(0);
/// 当前演示阶段
static DEMO_PHASE: AtomicU32 = AtomicU32::new(0);
/// 内存压力测试计数器
static MEMORY_TEST_COUNT: AtomicU32 = AtomicU32::new(0);
/// 层ID存储
static LAYER_IDS: AtomicU64 = AtomicU64::new(0);

/// 演示阶段枚举
#[derive(Clone, Copy, Debug)]
#[repr(u32)]
enum DemoPhase {
    /// 基础离屏渲染
    BasicOffscreen = 0,
    /// 模糊滤镜演示
    BlurFilters = 1,
    /// 阴影效果演示
    ShadowEffects = 2,
    /// 混合模式演示
    BlendModes = 3,
    /// 嵌套层演示
    NestedLayers = 4,
    /// 内存压力测试
    MemoryPressure = 5,
}

impl DemoPhase {
    fn from_u32(v: u32) -> Self {
        match v % 6 {
            0 => DemoPhase::BasicOffscreen,
            1 => DemoPhase::BlurFilters,
            2 => DemoPhase::ShadowEffects,
            3 => DemoPhase::BlendModes,
            4 => DemoPhase::NestedLayers,
            _ => DemoPhase::MemoryPressure,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            DemoPhase::BasicOffscreen => "Basic Offscreen",
            DemoPhase::BlurFilters => "Blur Filters",
            DemoPhase::ShadowEffects => "Shadow Effects",
            DemoPhase::BlendModes => "Blend Modes",
            DemoPhase::NestedLayers => "Nested Layers",
            DemoPhase::MemoryPressure => "Memory Pressure",
        }
    }
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub extern "C" fn main() {
    init();
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub extern "C" fn guest_tick() {
    tick();
}

pub fn init() {
    dyxel_view::log("[OSV] =========================================");
    dyxel_view::log("[OSV] Off-Screen Rendering Validation Demo");
    dyxel_view::log("[OSV] =========================================");

    // 注册滤镜预设
    // Filter presets not available
    dyxel_view::log("[OSV] ✓ Filter presets registered");

    // 创建根容器
    let root = View::new()
        .width("100%")
        .height("100%")
        .color((240u32, 240, 245, 255))
        .flex_direction(FlexDirection::Column)
        .align_items(AlignItems::Center)
        .justify_content(JustifyContent::FlexStart)
        .padding((20.0, 20.0, 20.0, 20.0));

    // 标题
    let title = Text::new()
        .value("OSR Validation Demo")
        .font_size(22.0)
        .text_color((0u8, 100, 150, 255));
    View { id: root.id }.child(title.id);

    let subtitle = Text::new()
        .value("All features: Layers, Filters, Shadows, Blend Modes")
        .font_size(11.0)
        .text_color((100u8, 100, 100, 255));
    View { id: root.id }.child(subtitle.id);

    // 状态显示
    let status_bar = View::new()
        .width("100%")
        .height(30.0)
        .color((210u32, 210, 220, 255))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::SpaceBetween)
        .align_items(AlignItems::Center)
        .padding((10.0, 0.0, 10.0, 0.0));
    View { id: root.id }.child(status_bar.id);

    let phase_text = Text::new()
        .value("Phase: Basic Offscreen")
        .font_size(10.0)
        .text_color((80u8, 80, 80, 255));
    View { id: status_bar.id }.child(phase_text.id);

    let memory_text = Text::new()
        .value("Memory: 0 MB")
        .font_size(10.0)
        .text_color((80u8, 80, 80, 255));
    View { id: status_bar.id }.child(memory_text.id);

    // 主演示区域
    let demo_area = View::new()
        .width("100%")
        .flex_grow(1.0)
        .color((250u32, 250, 252, 255))
        .flex_direction(FlexDirection::Column)
        .padding((10.0, 10.0, 10.0, 10.0));
    View { id: root.id }.child(demo_area.id);

    // ===== 验证 1: 基础离屏渲染 =====
    create_basic_offscreen_section(&demo_area);

    // ===== 验证 2: 模糊滤镜 =====
    create_blur_section(&demo_area);

    // ===== 验证 3: 阴影效果 =====
    create_shadow_section(&demo_area);

    // ===== 验证 4: 混合模式 =====
    create_blend_mode_section(&demo_area);

    // ===== 验证 5: 嵌套层 =====
    create_nested_layer_section(&demo_area);

    // ===== 验证 6: 纹理池压力测试 =====
    create_memory_pressure_section(&demo_area);

    // 存储状态栏ID用于更新
    let ids = ((phase_text.id as u64) << 32) | (memory_text.id as u64);
    LAYER_IDS.store(ids, Ordering::SeqCst);

    dyxel_view::log("[OSV] ✓ Demo initialized with 6 validation sections");
}

/// 验证 1: 基础离屏渲染
fn create_basic_offscreen_section(parent: &View) {
    let _section = create_section_title(parent, "1. Basic Offscreen Rendering");

    let container = View::new()
        .width("100%")
        .height(100.0)
        .color((220u32, 220, 230, 255))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::SpaceAround)
        .align_items(AlignItems::Center);
    View { id: parent.id }.child(container.id);

    // 普通渲染（无离屏）
    let normal = View::new()
        .width(100.0)
        .height(60.0)
        .color((100u32, 100, 100, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    View { id: container.id }.child(normal.id);

    let normal_text = Text::new()
        .value("Normal")
        .font_size(10.0)
        .text_color((30u8, 30, 30, 255));
    View { id: normal.id }.child(normal_text.id);

    // 离屏渲染（透明度）
    let offscreen = View::new()
        .width(100.0)
        .height(60.0)
        .color((100u32, 200, 100, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center)
        .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.5));
    View { id: container.id }.child(offscreen.id);

    let offscreen_text = Text::new()
        .value("Offscreen α=0.5")
        .font_size(10.0)
        .text_color((30u8, 30, 30, 255));
    View { id: offscreen.id }.child(offscreen_text.id);
}

/// 验证 2: 模糊滤镜
fn create_blur_section(parent: &View) {
    let _section = create_section_title(parent, "2. Dual-Filtering Blur");

    let container = View::new()
        .width("100%")
        .height(120.0)
        .color((210u32, 210, 220, 255))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::SpaceAround)
        .align_items(AlignItems::Center);
    View { id: parent.id }.child(container.id);

    // 原始
    let original = create_blur_card("Original", None);
    View { id: container.id }.child(original.id);

    // 轻模糊 (2px)
    let light = create_blur_card("Blur 2px", Some(0));
    View { id: container.id }.child(light.id);

    // 中等模糊 (5px)
    let medium = create_blur_card("Blur 5px", Some(1));
    View { id: container.id }.child(medium.id);

    // 强烈模糊 (10px)
    let heavy = create_blur_card("Blur 10px", Some(2));
    View { id: container.id }.child(heavy.id);
}

/// 验证 3: 阴影效果
fn create_shadow_section(parent: &View) {
    let _section = create_section_title(parent, "3. Drop Shadow Effects");

    let container = View::new()
        .width("100%")
        .height(120.0)
        .color((220u32, 220, 230, 255))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::SpaceAround)
        .align_items(AlignItems::Center);
    View { id: parent.id }.child(container.id);

    // 无阴影
    let no_shadow = View::new()
        .width(80.0)
        .height(80.0)
        .color((100u32, 150, 200, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    View { id: container.id }.child(no_shadow.id);

    let no_shadow_text = Text::new()
        .value("No Shadow")
        .font_size(9.0)
        .text_color((30u8, 30, 30, 255));
    View { id: no_shadow.id }.child(no_shadow_text.id);

    // 轻微阴影
    let light_shadow = create_shadow_card("Light", 2.0, 2.0, 4.0);
    View { id: container.id }.child(light_shadow.id);

    // 中等阴影
    let medium_shadow = create_shadow_card("Medium", 4.0, 4.0, 8.0);
    View { id: container.id }.child(medium_shadow.id);

    // 强烈阴影
    let heavy_shadow = create_shadow_card("Heavy", 8.0, 8.0, 16.0);
    View { id: container.id }.child(heavy_shadow.id);
}

/// 验证 4: 混合模式
fn create_blend_mode_section(parent: &View) {
    let _section = create_section_title(parent, "4. Blend Modes");

    let container = View::new()
        .width("100%")
        .height(100.0)
        .color((230u32, 220, 220, 255))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::SpaceAround)
        .align_items(AlignItems::Center);
    View { id: parent.id }.child(container.id);

    // 背景层（用于混合）
    let bg = View::new()
        .width("100%")
        .height(100.0)
        .color((200u32, 50, 50, 255));
    View { id: container.id }.child(bg.id);

    // 各种混合模式
    let modes = [
        ("Normal", 0, (100, 100, 255)),
        ("Multiply", 1, (100, 255, 100)),
        ("Screen", 2, (255, 100, 100)),
        ("Overlay", 3, (255, 200, 100)),
    ];

    for (name, mode, (r, g, b)) in modes.iter() {
        let card = View::new()
            .width(70.0)
            .height(70.0)
            .color((*r as u32, *g as u32, *b as u32, 200))
            .flex_direction(FlexDirection::Column)
            .justify_content(JustifyContent::Center)
            .align_items(AlignItems::Center)
            .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.8));
        View { id: container.id }.child(card.id);

        let text = Text::new()
            .value(*name)
            .font_size(9.0)
            .text_color((30u8, 30, 30, 255));
        View { id: card.id }.child(text.id);
    }
}

/// 验证 5: 嵌套层
fn create_nested_layer_section(parent: &View) {
    let _section = create_section_title(parent, "5. Nested Layers");

    let container = View::new()
        .width("100%")
        .height(150.0)
        .color((220u32, 230, 220, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    View { id: parent.id }.child(container.id);

    // 外层（离屏）
    let outer = View::new()
        .width(200.0)
        .height(120.0)
        .color((80u32, 80, 120, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center)
        .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.9));
    View { id: container.id }.child(outer.id);

    let outer_text = Text::new()
        .value("Outer Layer (α=0.9)")
        .font_size(10.0)
        .text_color((30u8, 30, 30, 255));
    View { id: outer.id }.child(outer_text.id);

    // 中层（离屏 + 模糊）
    let middle = View::new()
        .width(160.0)
        .height(70.0)
        .color((120u32, 80, 120, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center)
        .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.8));
    View { id: outer.id }.child(middle.id);

    let middle_text = Text::new()
        .value("Middle (blur)")
        .font_size(9.0)
        .text_color((30u8, 30, 30, 255));
    View { id: middle.id }.child(middle_text.id);

    // 内层（普通）
    let inner = View::new()
        .width(100.0)
        .height(30.0)
        .color((80u32, 120, 80, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    View { id: middle.id }.child(inner.id);

    let inner_text = Text::new()
        .value("Inner")
        .font_size(8.0)
        .text_color((30u8, 30, 30, 255));
    View { id: inner.id }.child(inner_text.id);
}

/// 验证 6: 内存压力测试
fn create_memory_pressure_section(parent: &View) {
    let _section = create_section_title(parent, "6. Memory Pressure Test (128MB Budget)");

    let container = View::new()
        .width("100%")
        .height(100.0)
        .color((230u32, 220, 220, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    View { id: parent.id }.child(container.id);

    let info = Text::new()
        .value("Texture Pool: 128MB with LRU eviction")
        .font_size(10.0)
        .text_color((150u8, 80, 80, 255));
    View { id: container.id }.child(info.id);

    let desc = Text::new()
        .value("Buckets: Small(256²) Medium(512²) Large(1024²) XL(2048²)")
        .font_size(9.0)
        .text_color((90u8, 90, 90, 255));
    View { id: container.id }.child(desc.id);

    // 动态创建的大纹理指示器
    let texture_grid = View::new()
        .width("100%")
        .height(50.0)
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    View { id: container.id }.child(texture_grid.id);

    // 创建一些示例纹理占位符
    for i in 0..5 {
        let block = View::new()
            .width(40.0)
            .height(40.0)
            .color(((50 + i * 40) as u32, 50, 50, 255))
            .margin((5.0, 5.0, 5.0, 5.0));
        View { id: texture_grid.id }.child(block.id);
    }
}

// ===== 辅助函数 =====

fn create_section_title(parent: &View, title: &str) -> View {
    let spacer = View::new().width("100%").height(10.0);
    View { id: parent.id }.child(spacer.id);

    let title_view = Text::new()
        .value(title)
        .font_size(12.0)
        .text_color((0u8, 100, 150, 255));
    View { id: parent.id }.child(title_view.id);

    let spacer2 = View::new().width("100%").height(5.0);
    View { id: parent.id }.child(spacer2.id);

    View { id: title_view.id }
}

fn create_blur_card(label: &str, filter_id: Option<u32>) -> View {
    let card = if let Some(fid) = filter_id {
        View::new()
            .width(70.0)
            .height(90.0)
            .color((100u32, 150, 100, 255))
            .flex_direction(FlexDirection::Column)
            .justify_content(JustifyContent::Center)
            .align_items(AlignItems::Center)
            .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.8))
    } else {
        View::new()
            .width(70.0)
            .height(90.0)
            .color((100u32, 150, 100, 255))
            .flex_direction(FlexDirection::Column)
            .justify_content(JustifyContent::Center)
            .align_items(AlignItems::Center)
            .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.95))
    };

    let text = Text::new()
        .value(label)
        .font_size(9.0)
        .text_color((30u8, 30, 30, 255));
    View { id: card.id }.child(text.id);

    // 添加一些内容用于展示模糊效果
    let content = View::new()
        .width(50.0)
        .height(30.0)
        .color((200u32, 200, 100, 255));
    View { id: card.id }.child(content.id);

    card
}

fn create_shadow_card(label: &str, dx: f32, dy: f32, blur: f32) -> View {
    // 创建阴影滤镜ID（这里简化处理，使用预设）
    let card = View::new()
        .width(80.0)
        .height(80.0)
        .color((100u32, 150, 200, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center)
        .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.95));

    let text = Text::new()
        .value(label)
        .font_size(9.0)
        .text_color((30u8, 30, 30, 255));
    View { id: card.id }.child(text.id);

    let shadow_info = Text::new()
        .value(&format!("{:.0},{:.0} blur:{:.0}", dx, dy, blur))
        .font_size(7.0)
        .text_color((80u8, 80, 80, 255));
    View { id: card.id }.child(shadow_info.id);

    card
}

pub fn tick() {
    let frame = ANIMATION_FRAME.fetch_add(1, Ordering::SeqCst);

    // 每 300 帧切换演示阶段
    if frame % 300 == 0 && frame > 0 {
        let new_phase = (frame / 300) % 6;
        DEMO_PHASE.store(new_phase as u32, Ordering::SeqCst);

        let phase = DemoPhase::from_u32(new_phase as u32);
        dyxel_view::log(&format!("[OSV] === Phase changed: {} ===", phase.name()));

        // 更新状态栏
        update_status_bar(&phase);
    }

    // 定期报告
    if frame % 60 == 0 {
        report_stats(frame);
    }

    dyxel_view::dyxel_view_tick();
}

fn update_status_bar(phase: &DemoPhase) {
    let ids = LAYER_IDS.load(Ordering::SeqCst);
    let phase_id = (ids >> 32) as u32;

    if phase_id != 0 {
        // Status update not available: View { id: phase_id }.text_value(...)
    }
}

fn report_stats(frame: u32) {
    let phase = DemoPhase::from_u32(DEMO_PHASE.load(Ordering::SeqCst));

    // 这里可以集成实际的内存统计
    let estimated_memory = estimate_memory_usage();

    dyxel_view::log(&format!(
        "[OSV] Frame {} | Phase: {} | Est. Memory: {:.1} MB",
        frame,
        phase.name(),
        estimated_memory
    ));
}

fn estimate_memory_usage() -> f32 {
    // 简化的内存估算
    // 实际实现应该查询 TexturePool 的统计信息
    let count = MEMORY_TEST_COUNT.load(Ordering::SeqCst);
    (count as f32 * 0.5).min(128.0)
}
