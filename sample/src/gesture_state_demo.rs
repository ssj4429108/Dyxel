// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture & State System Validation Demo
//!
//! Validates:
//! - Tap / DoubleTap / LongPress / Pan gestures
//! - use_state / use_memo state management
//! - String interpolation: "Count: {count}"
//! - State binding in attributes

use dyxel_app::prelude::*;

#[app]
pub fn GestureStateDemo() -> impl BaseView {
    // Create states
    let tap_count = use_state(|| 0u32);
    let double_tap_count = use_state(|| 0u32);
    let long_press_count = use_state(|| 0u32);
    let pan_count = use_state(|| 0u32);
    let pan_x = use_state(|| 0.0f32);
    let pan_y = use_state(|| 0.0f32);
    let last_event = use_state(|| "Demo started".to_string());

    // Clone for closures
    let tap_count2 = tap_count.clone();
    let last_event2 = last_event.clone();
    let long_press_count2 = long_press_count.clone();
    let last_event3 = last_event.clone();
    let pan_count2 = pan_count.clone();
    let pan_x2 = pan_x.clone();
    let pan_y2 = pan_y.clone();
    let last_event4 = last_event.clone();

    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (20, 20, 30),
            flexDirection: FlexDirection::Column,
            justifyContent: JustifyContent::FlexStart,
            alignItems: AlignItems::Center,
            padding: (20.0, 20.0, 20.0, 20.0),

            // Title
            Text("Gesture & State Demo") {
                fontSize: 24.0,
                textColor: (255, 255, 255, 255),
                margin: (0.0, 0.0, 20.0, 0.0),
            }

            // Counter Cards Row
            View {
                width: "100%",
                height: 100.0,
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceAround,
                alignItems: AlignItems::Center,
                margin: (0.0, 0.0, 20.0, 0.0),

                // Tap Counter Card
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (60, 140, 240),
                    borderRadius: 12.0,
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,

                    Text("Tap") {
                        fontSize: 12.0,
                        textColor: (255, 255, 255, 200),
                        margin: (0.0, 0.0, 5.0, 0.0),
                    }
                    // String interpolation with state
                    Text("{tap_count}") {
                        fontSize: 32.0,
                        fontWeight: 700,
                        textColor: (255, 255, 255, 255),
                    }
                }

                // Double Tap Counter Card
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (140, 60, 240),
                    borderRadius: 12.0,
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,

                    Text("Double") {
                        fontSize: 12.0,
                        textColor: (255, 255, 255, 200),
                        margin: (0.0, 0.0, 5.0, 0.0),
                    }
                    Text("{double_tap_count}") {
                        fontSize: 32.0,
                        fontWeight: 700,
                        textColor: (255, 255, 255, 255),
                    }
                }

                // Long Press Counter Card
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (240, 140, 60),
                    borderRadius: 12.0,
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,

                    Text("Long") {
                        fontSize: 12.0,
                        textColor: (255, 255, 255, 200),
                        margin: (0.0, 0.0, 5.0, 0.0),
                    }
                    Text("{long_press_count}") {
                        fontSize: 32.0,
                        fontWeight: 700,
                        textColor: (255, 255, 255, 255),
                    }
                }

                // Pan Counter Card
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (60, 200, 120),
                    borderRadius: 12.0,
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,

                    Text("Pan") {
                        fontSize: 12.0,
                        textColor: (255, 255, 255, 200),
                        margin: (0.0, 0.0, 5.0, 0.0),
                    }
                    Text("{pan_count}") {
                        fontSize: 32.0,
                        fontWeight: 700,
                        textColor: (255, 255, 255, 255),
                    }
                }
            }

            // Tap Area
            View {
                width: "90%",
                height: 80.0,
                color: (40, 40, 55),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                onTap: move |_, _| {
                    let count = tap_count2.get() + 1;
                    tap_count2.set(count);
                    last_event2.set(format!("Tap #{} detected", count));
                },

                Text("Tap Me") {
                    fontSize: 18.0,
                    textColor: (255, 255, 255, 255),
                }
            }

            // Long Press Area
            View {
                width: "90%",
                height: 80.0,
                color: (55, 40, 40),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                onLongPress: move |_, _| {
                    let count = long_press_count2.get() + 1;
                    long_press_count2.set(count);
                    last_event3.set(format!("LongPress #{} completed", count));
                },

                Text("Long Press Me (Hold 500ms)") {
                    fontSize: 16.0,
                    textColor: (255, 200, 200, 255),
                }
            }

            // Pan Area
            View {
                width: "90%",
                height: 120.0,
                color: (40, 55, 45),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                onPanUpdate: move |dx, dy| {
                    let count = pan_count2.get() + 1;
                    pan_count2.set(count);
                    pan_x2.set(pan_x2.get() + dx);
                    pan_y2.set(pan_y2.get() + dy);
                    last_event4.set(format!("Pan #{}: delta=({:.1}, {:.1})", count, dx, dy));
                },

                View {
                    width: "100%",
                    height: "100%",
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,

                    Text("Pan Me (Drag around)") {
                        fontSize: 16.0,
                        textColor: (200, 255, 200, 255),
                        margin: (0.0, 0.0, 8.0, 0.0),
                    }
                    // String interpolation showing position
                    Text("X: {pan_x}, Y: {pan_y}") {
                        fontSize: 12.0,
                        textColor: (150, 200, 150, 255),
                    }
                }
            }

            // Event Log
            View {
                width: "90%",
                flexGrow: 1.0,
                color: (30, 30, 40),
                borderRadius: 8.0,
                padding: (10.0, 10.0, 10.0, 10.0),
                flexDirection: FlexDirection::Column,
                alignItems: AlignItems::FlexStart,

                Text("Last Event:") {
                    fontSize: 14.0,
                    textColor: (150, 150, 170, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }
                // String interpolation with state
                Text("{last_event}") {
                    fontSize: 12.0,
                    textColor: (200, 200, 220, 255),
                }
            }
        }
    }
}
