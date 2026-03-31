// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Input Proxy Demo - Gesture and Input Validation Example
//!
//! Demo Features:
//! - Tap gesture
//! - Pan gesture
//! - Multi-touch (Android)
//! - Mouse wheel (macOS)
//! - Hot-area expansion test
//! - Event bubbling

use dyxel_view::{
    BaseView, FlexDirection, JustifyContent, AlignItems, Dimension,
    View, Text, set_text,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::cell::RefCell;

// Color definitions (RGB)
const COLOR_BG: (u32, u32, u32) = (20, 20, 30);
const COLOR_PANEL: (u32, u32, u32) = (40, 40, 55);
const COLOR_BUTTON: (u32, u32, u32) = (60, 120, 220);
const COLOR_BUTTON_ACTIVE: (u32, u32, u32) = (80, 160, 255);
const COLOR_TEXT: (u32, u32, u32) = (255, 255, 255);
const COLOR_TEXT_SECONDARY: (u32, u32, u32) = (180, 180, 200);
const COLOR_ACCENT: (u32, u32, u32) = (255, 100, 100);
const COLOR_SUCCESS: (u32, u32, u32) = (100, 255, 150);

// Counters for interaction display
static TAP_COUNTER: AtomicU32 = AtomicU32::new(0);
static PAN_COUNTER: AtomicU32 = AtomicU32::new(0);

/// UI State - stores node IDs for dynamic updates
struct UIState {
    // Tap demo
    tap_counter_id: u32,
    
    // Pan demo
    position_text_id: u32,
    pan_status_id: u32,
    
    // Log panel
    log_line_ids: [u32; 5],
    last_log_count: u32,
}

impl Default for UIState {
    fn default() -> Self {
        Self {
            tap_counter_id: 0,
            position_text_id: 0,
            pan_status_id: 0,
            log_line_ids: [0; 5],
            last_log_count: 0,
        }
    }
}

thread_local! {
    static UI_STATE: RefCell<UIState> = RefCell::new(UIState::default());
    // Pan state
    static PAN_STATE: RefCell<PanState> = RefCell::new(PanState::default());
    // Log messages
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

/// Add log message
fn add_log(msg: String) {
    LOG_MESSAGES.with(|logs| {
        let mut logs = logs.borrow_mut();
        logs.push(msg);
        // Keep only last 5 entries
        while logs.len() > 5 {
            logs.remove(0);
        }
    });
}

/// Initialize demo application
pub fn init() {
    // Create root container
    let root = View::new()
        .width(Dimension::Percent(100.0))
        .height(Dimension::Percent(100.0))
        .color(COLOR_BG)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::FlexStart)
        .align_items(AlignItems::Center);
    
    // Title
    let title = Text::new()
        .value("Input Proxy Demo")
        .font_size(24.0);
    View { id: root.node_id() }.child(title.node_id());
    
    // Subtitle
    let subtitle = Text::new()
        .value("Gesture Recognition & Input Validation")
        .font_size(14.0);
    View { id: root.node_id() }.child(subtitle.node_id());
    
    // Create demo areas
    let tap_panel = create_tap_demo();
    let pan_panel = create_pan_demo();
    let small_target_panel = create_small_target_demo();
    let log_panel = create_log_panel();
    
    // Add child nodes
    View { id: root.node_id() }.child(tap_panel.node_id());
    View { id: root.node_id() }.child(pan_panel.node_id());
    View { id: root.node_id() }.child(small_target_panel.node_id());
    View { id: root.node_id() }.child(log_panel.node_id());
    
    // Platform hint
    let platform_hint = if cfg!(target_os = "android") {
        "Android: Try multi-touch"
    } else if cfg!(target_os = "macos") {
        "macOS: Try mouse wheel and drag"
    } else {
        "Web: Touch or mouse input"
    };
    
    let hint = Text::new()
        .value(platform_hint)
        .font_size(12.0);
    View { id: root.node_id() }.child(hint.node_id());
    
    add_log("App started".to_string());
    add_log("Tap the blue button".to_string());
}

/// Create Tap gesture demo area
fn create_tap_demo() -> View {
    // Container
    let panel = View::new()
        .width(Dimension::Pixels(300.0))
        .height(Dimension::Pixels(120.0))
        .color(COLOR_PANEL)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    // Title
    let title = Text::new()
        .value("Tap Gesture Test")
        .font_size(16.0);
    View { id: panel.node_id() }.child(title.node_id());
    
    // Tap button (large target)
    let tap_button = View::new()
        .width(200.0)
        .height(50.0)
        .color(COLOR_BUTTON)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    let button_text = Text::new()
        .value("Tap Me")
        .font_size(16.0);
    View { id: tap_button.node_id() }.child(button_text.node_id());
    
    // Add button to panel first, then set click callback (on_click consumes tap_button)
    let tap_button_id = tap_button.node_id();
    View { id: panel.node_id() }.child(tap_button_id);
    
    // Click callback
    tap_button.on_click({
        let counter = &TAP_COUNTER;
        move || {
            let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
            add_log(format!("Tap #{} detected", count));
        }
    });
    
    // Counter display - store ID for dynamic update
    let counter_text = Text::new()
        .value("Taps: 0")
        .font_size(12.0);
    let counter_id = counter_text.node_id();
    View { id: panel.node_id() }.child(counter_id);
    
    // Store ID for dynamic updates
    UI_STATE.with(|state| {
        state.borrow_mut().tap_counter_id = counter_id;
    });
    
    panel
}

/// Create Pan gesture demo area
fn create_pan_demo() -> View {
    let panel = View::new()
        .width(Dimension::Pixels(300.0))
        .height(Dimension::Pixels(150.0))
        .color(COLOR_PANEL)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    // Title
    let title = Text::new()
        .value("Pan Gesture Test")
        .font_size(16.0);
    View { id: panel.node_id() }.child(title.node_id());
    
    // Draggable area
    let drag_area = View::new()
        .width(Dimension::Pixels(280.0))
        .height(Dimension::Pixels(80.0))
        .color(COLOR_BUTTON)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    let drag_hint = Text::new()
        .value("Drag in this area")
        .font_size(14.0);
    View { id: drag_area.node_id() }.child(drag_hint.node_id());
    
    // Position display - store ID
    let position_text = Text::new()
        .value("Position: (0, 0)")
        .font_size(12.0);
    let position_id = position_text.node_id();
    View { id: drag_area.node_id() }.child(position_id);
    
    View { id: panel.node_id() }.child(drag_area.node_id());
    
    // Status display - store ID
    let state_text = Text::new()
        .value("Status: Waiting for drag...")
        .font_size(12.0);
    let status_id = state_text.node_id();
    View { id: panel.node_id() }.child(status_id);
    
    // Store IDs for dynamic updates
    UI_STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.position_text_id = position_id;
        state.pan_status_id = status_id;
    });
    
    panel
}

/// Create small target hot-area expansion test
fn create_small_target_demo() -> View {
    let panel = View::new()
        .width(Dimension::Pixels(300.0))
        .height(Dimension::Pixels(100.0))
        .color(COLOR_PANEL)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    // Title
    let title = Text::new()
        .value("Hot-area Test (20x20dp)")
        .font_size(14.0);
    View { id: panel.node_id() }.child(title.node_id());
    
    // Small button (20x20, smaller than 44dp min target)
    let small_button = View::new()
        .width(Dimension::Pixels(20.0))
        .height(Dimension::Pixels(20.0))
        .color(COLOR_ACCENT)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::Center)
        .align_items(AlignItems::Center);
    
    let small_button_id = small_button.node_id();
    View { id: panel.node_id() }.child(small_button_id);
    
    // Click callback
    small_button.on_click({
        move || {
            add_log("Small button tapped!".to_string());
        }
    });
    
    // Hint text
    let hint = Text::new()
        .value("Tap the red square (8dp hot-area around it)")
        .font_size(10.0);
    View { id: panel.node_id() }.child(hint.node_id());
    
    panel
}

/// Create log panel
fn create_log_panel() -> View {
    let panel = View::new()
        .width(Dimension::Pixels(300.0))
        .height(Dimension::Pixels(150.0))
        .color(COLOR_PANEL)
        .flex_direction(FlexDirection::Column)
        .justify_content(JustifyContent::FlexStart)
        .align_items(AlignItems::Center);
    
    // Title
    let title = Text::new()
        .value("Event Log")
        .font_size(14.0);
    View { id: panel.node_id() }.child(title.node_id());
    
    // Log content area - store IDs for dynamic updates
    let mut log_ids = [0u32; 5];
    for i in 0..5 {
        let log_line = Text::new()
            .value(&format!("{}. Waiting...", i + 1))
            .font_size(10.0);
        let line_id = log_line.node_id();
        log_ids[i] = line_id;
        View { id: panel.node_id() }.child(line_id);
    }
    
    // Store IDs for dynamic updates
    UI_STATE.with(|state| {
        state.borrow_mut().log_line_ids = log_ids;
    });
    
    panel
}

/// Per-frame update - dynamically update UI
pub fn tick() {
    // Get current values
    let tap_count = TAP_COUNTER.load(Ordering::SeqCst);
    
    UI_STATE.with(|state| {
        let state = state.borrow();
        
        // Update tap counter display
        if state.tap_counter_id != 0 {
            set_text(state.tap_counter_id, &format!("Taps: {}", tap_count));
        }
        
        // Update pan display (simulate pan data for now)
        PAN_STATE.with(|pan| {
            let pan = pan.borrow();
            if state.position_text_id != 0 {
                set_text(state.position_text_id, 
                    &format!("Position: ({:.0}, {:.0})", pan.current_x, pan.current_y));
            }
            if state.pan_status_id != 0 {
                let status = if pan.is_dragging {
                    "Status: Dragging..."
                } else {
                    "Status: Waiting for drag..."
                };
                set_text(state.pan_status_id, status);
            }
        });
        
        // Update log display
        LOG_MESSAGES.with(|logs| {
            let logs = logs.borrow();
            for (i, &line_id) in state.log_line_ids.iter().enumerate() {
                if line_id != 0 {
                    let msg = if i < logs.len() {
                        &logs[logs.len() - 1 - i]
                    } else {
                        ""
                    };
                    set_text(line_id, msg);
                }
            }
        });
    });
}

/// Platform info
pub fn get_platform_info() -> &'static str {
    if cfg!(target_os = "android") {
        "Android - Multi-touch, pressure support"
    } else if cfg!(target_os = "macos") {
        "macOS - Mouse wheel, precise pointer"
    } else if cfg!(target_os = "ios") {
        "iOS - Multi-touch support"
    } else {
        "Web/Unknown"
    }
}
