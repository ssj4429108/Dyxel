// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Off-Screen Rendering (OSR) 演示示例
//!
//! 本示例演示离屏渲染的核心功能：
//! - 图层隔离与半透明效果
//! - 图层透明度动画
//! - 光栅缓存控制
//! - 高斯模糊滤镜效果
//! - 阴影效果

use dyxel_view::{BaseView, FlexDirection, JustifyContent, AlignItems, 
    View, Text, ViewOffscreenExt, CacheHint,
    FilterRegistry, FilterId, ViewFilterExt, BlendMode, filter_presets,
};
use std::sync::atomic::{AtomicU32, Ordering};

/// 动画帧计数
static ANIMATION_FRAME: AtomicU32 = AtomicU32::new(0);
/// 当前透明度值
static CURRENT_ALPHA: AtomicU32 = AtomicU32::new(0.5f32.to_bits());
/// 淡入淡出图层 ID
static FADE_LAYER_ID: AtomicU32 = AtomicU32::new(0);

pub fn init() {
    // 初始化 ShadowTree

    // 注册滤镜预设
    filter_presets::register_presets();
    dyxel_view::log("[OSR] 滤镜预设已注册: Light(2px), Medium(5px), Heavy(10px) blur");

    // 创建根容器
    let root = View::new()
        .width("100%")
        .height("100%")
        .color((10u32, 10, 20, 255))
        .flex_direction(FlexDirection::Column)
        .align_items(AlignItems::Center)
        .justify_content(JustifyContent::FlexStart)
        .padding((20.0, 20.0, 20.0, 20.0));

    // ===== 标题 =====
    let title = Text::new()
        .value("Off-Screen Rendering Demo")
        .font_size(20.0)
        .text_color((255u8, 255, 255, 255));
    View { id: root.id }.child(title.id);

    let subtitle = Text::new()
        .value("Layer isolation, blur filters & raster cache")
        .font_size(12.0)
        .text_color((180u8, 180, 180, 255));
    View { id: root.id }.child(subtitle.id);

    let spacer = View::new().width("100%").height(20.0);
    View { id: root.id }.child(spacer.id);

    // ===== 示例 1：半透明动画卡片 =====
    create_alpha_demo(&root);

    let spacer2 = View::new().width("100%").height(20.0);
    View { id: root.id }.child(spacer2.id);

    // ===== 示例 2：光栅缓存演示 =====
    create_cache_demo(&root);

    let spacer3 = View::new().width("100%").height(20.0);
    View { id: root.id }.child(spacer3.id);

    // ===== 示例 3：模糊滤镜演示 =====
    create_blur_demo(&root);

    dyxel_view::log("[OSR] 离屏渲染演示已初始化");
}

/// 示例 1：半透明动画
fn create_alpha_demo(parent: &View) {
    let section_title = Text::new()
        .value("1. Alpha Blending (Animated)")
        .font_size(14.0)
        .text_color((100u8, 200, 255, 255));
    View { id: parent.id }.child(section_title.id);

    // 背景条纹容器
    let background = View::new()
        .width("100%")
        .height(150.0)
        .color((30u32, 30, 50, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    View { id: parent.id }.child(background.id);

    // 创建条纹背景
    for i in 0..7 {
        let stripe = View::new()
            .width("100%")
            .height(18.0)
            .color(if i % 2 == 0 { (60u32, 60, 80, 255) } else { (40u32, 40, 60, 255) });
        View { id: background.id }.child(stripe.id);
    }

    // 半透明覆盖层（离屏渲染）
    let overlay = View::new()
        .width(200.0)
        .height(80.0)
        .color((255u32, 100, 100, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center)
        .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.5));

    FADE_LAYER_ID.store(overlay.id, Ordering::SeqCst);
    View { id: parent.id }.child(overlay.id);

    let overlay_text = Text::new()
        .value("Fading Overlay")
        .font_size(14.0)
        .text_color((255u8, 255, 255, 255));
    View { id: overlay.id }.child(overlay_text.id);

    let alpha_label = Text::new()
        .value("alpha: 0.50")
        .font_size(10.0)
        .text_color((255u8, 255, 255, 200));
    View { id: overlay.id }.child(alpha_label.id);
}

/// 示例 2：光栅缓存
fn create_cache_demo(parent: &View) {
    let section_title = Text::new()
        .value("2. Raster Cache Hint")
        .font_size(14.0)
        .text_color((100u8, 200, 255, 255));
    View { id: parent.id }.child(section_title.id);

    let container = View::new()
        .width("100%")
        .height(120.0)
        .color((25u32, 40, 25, 255))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::SpaceAround)
        .align_items(AlignItems::Center);
    View { id: parent.id }.child(container.id);

    // 禁用缓存卡片
    let no_cache = View::new()
        .width(120.0)
        .height(80.0)
        .color((200u32, 80, 80, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center)
        .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.9))
        .cache_hint(CacheHint::Disable);
    View { id: container.id }.child(no_cache.id);

    let no_cache_title = Text::new()
        .value("No Cache")
        .font_size(11.0)
        .text_color((255u8, 255, 255, 255));
    View { id: no_cache.id }.child(no_cache_title.id);

    let no_cache_desc = Text::new()
        .value("(Dynamic)")
        .font_size(9.0)
        .text_color((200u8, 200, 200, 255));
    View { id: no_cache.id }.child(no_cache_desc.id);

    // 启用缓存卡片
    let cached = View::new()
        .width(120.0)
        .height(80.0)
        .color((80u32, 200, 80, 255))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center)
        .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.9))
        .cache_hint(CacheHint::Enable);
    View { id: container.id }.child(cached.id);

    let cached_title = Text::new()
        .value("Cached")
        .font_size(11.0)
        .text_color((255u8, 255, 255, 255));
    View { id: cached.id }.child(cached_title.id);

    let cached_desc = Text::new()
        .value("(Static)")
        .font_size(9.0)
        .text_color((200u8, 200, 200, 255));
    View { id: cached.id }.child(cached_desc.id);
}

/// 示例 3：模糊滤镜
fn create_blur_demo(parent: &View) {
    let section_title = Text::new()
        .value("3. Blur Filter Effects")
        .font_size(14.0)
        .text_color((100u8, 200, 255, 255));
    View { id: parent.id }.child(section_title.id);

    let container = View::new()
        .width("100%")
        .height(120.0)
        .color((35u32, 35, 50, 255))
        .flex_direction(FlexDirection::Row)
        .justify_content(JustifyContent::SpaceAround)
        .align_items(AlignItems::Center);
    View { id: parent.id }.child(container.id);

    // 原始（无模糊）
    let original = create_filter_card("Original", "", (100u32, 200, 100, 255), None);
    View { id: container.id }.child(original.id);

    // 轻微模糊 (2px)
    let light_blur = create_filter_card("Light", "2px", (100u32, 200, 100, 255), Some(filter_presets::BLUR_LIGHT));
    View { id: container.id }.child(light_blur.id);

    // 中等模糊 (5px)
    let medium_blur = create_filter_card("Medium", "5px", (100u32, 200, 100, 255), Some(filter_presets::BLUR_MEDIUM));
    View { id: container.id }.child(medium_blur.id);

    // 强烈模糊 (10px)
    let heavy_blur = create_filter_card("Heavy", "10px", (100u32, 200, 100, 255), Some(filter_presets::BLUR_HEAVY));
    View { id: container.id }.child(heavy_blur.id);
}

/// 辅助函数：创建带滤镜的卡片
fn create_filter_card(label: &str, sublabel: &str, color: (u32, u32, u32, u32), filter_id: Option<FilterId>) -> View {
    let card = if let Some(fid) = filter_id {
        View::new()
            .width(70.0)
            .height(100.0)
            .color(color)
            .flex_direction(FlexDirection::Column)
            .justify_content(JustifyContent::Center)
            .align_items(AlignItems::Center)
            .offscreen_with_filter(0.95, fid, BlendMode::Normal)
    } else {
        View::new()
            .width(70.0)
            .height(100.0)
            .color(color)
            .flex_direction(FlexDirection::Column)
            .justify_content(JustifyContent::Center)
            .align_items(AlignItems::Center)
            .offscreen(dyxel_view::OffscreenConfig::with_alpha(0.95))
    };

    let title = Text::new()
        .value(label)
        .font_size(10.0)
        .text_color((255u8, 255, 255, 255));
    View { id: card.id }.child(title.id);

    if !sublabel.is_empty() {
        let sub = Text::new()
            .value(sublabel)
            .font_size(8.0)
            .text_color((200u8, 200, 200, 255));
        View { id: card.id }.child(sub.id);
    }

    card
}

pub fn tick() {
    let frame = ANIMATION_FRAME.fetch_add(1, Ordering::SeqCst);

    // 更新透明度动画
    update_alpha_animation(frame);

    // 定期报告状态
    if frame % 120 == 0 && frame > 0 {
        dyxel_view::log(&format!("[OSR] 第{}帧 - 当前透明度: {:.2}",
            frame, f32::from_bits(CURRENT_ALPHA.load(Ordering::SeqCst))));
    }

    dyxel_view::dyxel_view_tick();
}

/// 更新透明度动画
fn update_alpha_animation(frame: u32) {
    let cycle = (frame % 240) as f32 / 240.0;
    let alpha = (0.5 + 0.4 * (cycle * std::f32::consts::PI * 2.0).sin()).clamp(0.1, 0.9);

    let fade_id = FADE_LAYER_ID.load(Ordering::SeqCst);
    if fade_id != 0 {
        View { id: fade_id }.layer_alpha(alpha);
        CURRENT_ALPHA.store(alpha.to_bits(), Ordering::SeqCst);
    }
}
