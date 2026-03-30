// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Input Proxy Demo - 手势和输入验证示例
//!
//! 演示功能：
//! - Tap 点击手势
//! - Pan 拖动手势
//! - 多点触控（Android）
//! - 鼠标滚轮（macOS）
//! - 热区扩展测试
//! - 事件冒泡

use dyxel_view::{
    BaseView, FlexDirection, JustifyContent, AlignItems, Dimension,
    View, Text,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::cell::RefCell;

// 颜色定义 (RGB)
const COLOR_BG: (u32, u32, u32) = (20, 20, 30);
const COLOR_PANEL: (u32, u32, u32) = (40, 40, 55);
const COLOR_BUTTON: (u32, u32, u32) = (60, 120, 220);
const COLOR_BUTTON_ACTIVE: (u32, u32, u32) = (80, 160, 255);
const COLOR_TEXT: (u32, u32, u32) = (255, 255, 255);
const COLOR_TEXT_SECONDARY: (u32, u32, u32) = (180, 180, 200);
const COLOR_ACCENT: (u32, u32, u32) = (255, 100, 100);
const COLOR_SUCCESS: (u32, u32, u32) = (100, 255, 150);

// 计数器（用于显示交互次数）
static TAP_COUNTER: AtomicU32 = AtomicU32::new(0);
static PAN_COUNTER: AtomicU32 = AtomicU32::new(0);

thread_local! {
    // 拖动状态
    static PAN_STATE: RefCell<PanState> = RefCell::new(PanState::default());
    // 日志消息
    static LOG_MESSAGES: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

#[derive(Default, Clone)]
struct PanState {
    is_dragging: bool,
    start_x: f32,
    start_y: f32,
    current_x: f32,
    current_y: f32,
}

/// 打印日志（WASM 环境）
#[cfg(target_arch = "wasm32")]
fn log(msg: &str) {
    // WASM: 使用 host 提供的日志接口
    // 实际实现会通过 dyxel_view 的日志机制
    let _ = msg;
}

#[cfg(not(target_arch = "wasm32"))]
fn log(msg: &str) {
    println!("[InputProxyDemo] {}", msg);
}

/// 添加日志消息
fn add_log(msg: String) {
    LOG_MESSAGES.with(|logs| {
        let mut logs = logs.borrow_mut();
        logs.push(msg);
        // 只保留最近 10 条
        if logs.len() > 10 {
            logs.remove(0);
        }
    });
}

/// 初始化演示应用
pub fn init() {
    log("Input Proxy Demo initializing...");
    
    // 创建根容器
    let root = View::new()
        .width(Dimension::Percent(100.0))
        .height(Dimension::Percent(100.0))
        .color((20, 20, 30))
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::FlexStart)
        .align_items(AlignItems::Center);
    
    // 标题
    let title = Text::new()
        .value("🖐️ Input Proxy Demo")
        .font_size(24.0);
    View { id: root.node_id() }.child(title.node_id());
    
    // 副标题
    let subtitle = Text::new()
        .value("手势识别与输入验证")
        .font_size(14.0);
    View { id: root.node_id() }.child(subtitle.node_id());
    
    // 创建演示区域
    let tap_panel = create_tap_demo();
    let pan_panel = create_pan_demo();
    let small_target_panel = create_small_target_demo();
    let log_panel = create_log_panel();
    
    // 添加子节点
    View { id: root.node_id() }.child(tap_panel.node_id());
    View { id: root.node_id() }.child(pan_panel.node_id());
    View { id: root.node_id() }.child(small_target_panel.node_id());
    View { id: root.node_id() }.child(log_panel.node_id());
    
    // 平台提示
    let platform_hint = if cfg!(target_os = "android") {
        "📱 Android: 尝试多点触控"
    } else if cfg!(target_os = "macos") {
        "🖱️ macOS: 尝试鼠标滚轮和拖拽"
    } else {
        "🌐 Web: 触摸或鼠标输入"
    };
    
    let hint = Text::new()
        .value(platform_hint)
        .font_size(12.0);
    View { id: root.node_id() }.child(hint.node_id());
    
    log("Input Proxy Demo initialized");
    add_log("应用已启动".to_string());
}

/// 创建 Tap 点击演示区
fn create_tap_demo() -> View {
    // 容器
    let panel = View::new()
        .width(Dimension::Pixels(300.0))
        .height(Dimension::Pixels(120.0))
        .color(COLOR_PANEL)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    // 标题
    let title = Text::new()
        .value("👆 Tap 点击测试")
        .font_size(16.0);
    View { id: panel.node_id() }.child(title.node_id());
    
    // 点击按钮（大目标）
    let tap_button = View::new()
        .width(200.0)
        .height(50.0)
        .color(COLOR_BUTTON)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    let button_text = Text::new()
        .value("点击我")
        .font_size(16.0);
    View { id: tap_button.node_id() }.child(button_text.node_id());
    
    // 先添加按钮到面板，再设置点击回调（on_click 会消费 tap_button）
    let tap_button_id = tap_button.node_id();
    View { id: panel.node_id() }.child(tap_button_id);
    
    // 点击回调
    tap_button.on_click({
        let counter = &TAP_COUNTER;
        move || {
            let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
            let msg = format!("Tap 点击 #{} - 时间: {:?}", count, std::time::Instant::now());
            log(&msg);
            add_log(msg);
        }
    });
    
    // 计数器显示
    let counter_text = Text::new()
        .value("点击次数: 0")
        .font_size(12.0);
    View { id: panel.node_id() }.child(counter_text.node_id());
    
    panel
}

/// 创建 Pan 拖动演示区
fn create_pan_demo() -> View {
    let panel = View::new()
        .width(Dimension::Pixels(300.0))
        .height(Dimension::Pixels(150.0))
        .color(COLOR_PANEL)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    // 标题
    let title = Text::new()
        .value("✋ Pan 拖动测试")
        .font_size(16.0);
    View { id: panel.node_id() }.child(title.node_id());
    
    // 可拖动区域
    let drag_area = View::new()
        .width(Dimension::Pixels(280.0))
        .height(Dimension::Pixels(80.0))
        .color(COLOR_BUTTON)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    let drag_hint = Text::new()
        .value("在此区域拖动")
        .font_size(14.0);
    View { id: drag_area.node_id() }.child(drag_hint.node_id());
    
    // 位置显示
    let position_text = Text::new()
        .value("位置: (0, 0)")
        .font_size(12.0);
    View { id: drag_area.node_id() }.child(position_text.node_id());
    
    // TODO: 当 dyxel-view 支持 onPan 回调时添加
    // drag_area.on_pan_start(|x, y| { ... });
    // drag_area.on_pan_update(|x, y, dx, dy| { ... });
    // drag_area.on_pan_end(|x, y| { ... });
    
    View { id: panel.node_id() }.child(drag_area.node_id());
    
    // 状态显示
    let state_text = Text::new()
        .value("状态: 等待拖动...")
        .font_size(12.0);
    View { id: panel.node_id() }.child(state_text.node_id());
    
    panel
}

/// 创建小目标热区扩展测试
fn create_small_target_demo() -> View {
    let panel = View::new()
        .width(Dimension::Pixels(300.0))
        .height(Dimension::Pixels(100.0))
        .color(COLOR_PANEL)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    // 标题
    let title = Text::new()
        .value("🎯 热区扩展测试 (20x20dp)")
        .font_size(14.0);
    View { id: panel.node_id() }.child(title.node_id());
    
    // 小按钮（20x20，小于 44dp 最小目标）
    // 注意：on_click 会消费 view，所以先添加子节点再设置点击
    let small_button = View::new()
        .width(Dimension::Pixels(20.0))
        .height(Dimension::Pixels(20.0))
        .color(COLOR_ACCENT)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    let small_button_id = small_button.node_id();
    View { id: panel.node_id() }.child(small_button_id);
    
    // 点击回调
    small_button.on_click({
        move || {
            let msg = "小按钮被点击！热区扩展生效".to_string();
            log(&msg);
            add_log(msg);
        }
    });
    
    // 提示文字
    let hint = Text::new()
        .value("尝试点击红色小方块（周围 8dp 也是热区）")
        .font_size(10.0);
    View { id: panel.node_id() }.child(hint.node_id());
    
    panel
}

/// 创建日志面板
fn create_log_panel() -> View {
    let panel = View::new()
        .width(Dimension::Pixels(300.0))
        .height(Dimension::Pixels(150.0))
        .color(COLOR_PANEL)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::FlexStart)
        .align_items(AlignItems::Center);
    
    // 标题
    let title = Text::new()
        .value("📝 事件日志")
        .font_size(14.0);
    View { id: panel.node_id() }.child(title.node_id());
    
    // 日志内容区域（简化显示）
    for i in 0..5 {
        let log_line = Text::new()
            .value(&format!("{}. 等待事件...", i + 1))
            .font_size(10.0);
        View { id: panel.node_id() }.child(log_line.node_id());
    }
    
    panel
}

/// 每帧更新
pub fn tick() {
    // 更新计数器显示
    let tap_count = TAP_COUNTER.load(Ordering::SeqCst);
    let pan_count = PAN_COUNTER.load(Ordering::SeqCst);
    
    // 在实际应用中，这里会更新 Text 节点的内容
    // 由于当前 API 限制，我们通过其他方式反馈
    
    // 每 60 帧输出一次状态（约 1 秒）
    static FRAME: AtomicU32 = AtomicU32::new(0);
    let frame = FRAME.fetch_add(1, Ordering::SeqCst);
    
    if frame % 60 == 0 && (tap_count > 0 || pan_count > 0) {
        log(&format!("Stats - Tap: {}, Pan: {}", tap_count, pan_count));
    }
}

/// 平台检测提示
pub fn get_platform_info() -> &'static str {
    if cfg!(target_os = "android") {
        "Android - 支持多点触控、压力感应"
    } else if cfg!(target_os = "macos") {
        "macOS - 支持鼠标滚轮、精确指针"
    } else if cfg!(target_os = "ios") {
        "iOS - 支持多点触控"
    } else {
        "Web/Unknown"
    }
}
