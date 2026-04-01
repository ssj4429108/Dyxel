// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! State Dynamic Binding Demo
//!
//! Validates:
//! - State binding to width/height: `width: {size.get()}`
//! - State binding to color: `color: {bg_color.get()}`
//! - State updates trigger UI re-layout

use dyxel_app::prelude::*;
use dyxel_view::log;

#[app]
pub fn StateBindingDemo() -> impl BaseView {
    // Create states for dynamic properties
    let box_width = use_state(|| 100.0f32);
    let box_height = use_state(|| 100.0f32);
    let bg_color = use_state(|| (60u32, 140, 240, 255));
    let counter = use_state(|| 0u32);

    // Clone for closures (Grow button)
    let box_width2 = box_width.clone();
    let box_height2 = box_height.clone();
    let counter2 = counter.clone();
    
    // Clone for closures (Shrink button)
    let box_width3 = box_width.clone();
    let box_height3 = box_height.clone();
    let counter3 = counter.clone();
    
    // Clone for closures (Color button)
    let counter4 = counter.clone();

    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (20u32, 20, 30, 255),
            flexDirection: FlexDirection::Column,
            justifyContent: JustifyContent::FlexStart,
            alignItems: AlignItems::Center,
            padding: (20.0, 20.0, 20.0, 20.0),

            // Title
            Text("State Dynamic Binding Demo") {
                fontSize: 24.0,
                textColor: (255, 255, 255, 255),
                margin: (0.0, 0.0, 20.0, 0.0),
            }

            // Dynamic Box - size and color bound to state
            View {
                // These should update when state changes
                // Use State directly (RSX macro auto-generates Signal binding)
                width: box_width,
                height: box_height,
                color: bg_color,
                borderRadius: 12.0,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                margin: (0.0, 0.0, 20.0, 0.0),

                Text("Dynamic Box") {
                    fontSize: 18.0,
                    textColor: (255, 255, 255, 255),
                }
            }

            // Counter Display
            Text("Changes: {counter}") {
                fontSize: 16.0,
                textColor: (200, 200, 200, 255),
                margin: (0.0, 0.0, 20.0, 0.0),
            }

            // Control Buttons
            View {
                width: "100%",
                height: 60.0,
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceAround,
                alignItems: AlignItems::Center,

                // Grow Button
                View {
                    width: 100.0,
                    height: 50.0,
                    color: (60u32, 180, 60, 255),
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    onTap: move |_| {
                        log("Grow button clicked!");
                        let old_width = box_width2.get();
                        let old_height = box_height2.get();
                        let new_width = (old_width + 20.0).min(300.0);
                        let new_height = (old_height + 20.0).min(300.0);
                        log(&format!("Grow: width {} -> {}, height {} -> {}", old_width, new_width, old_height, new_height));
                        box_width2.set(new_width);
                        box_height2.set(new_height);
                        counter2.set(counter2.get() + 1);
                        log("Grow: state updated");
                    },

                    Text("Grow") {
                        fontSize: 16.0,
                        textColor: (255, 255, 255, 255),
                    }
                }

                // Shrink Button
                View {
                    width: 100.0,
                    height: 50.0,
                    color: (180u32, 60, 60, 255),
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    onTap: move |_| {
                        log("Shrink button clicked!");
                        box_width3.set((box_width3.get() - 20.0).max(50.0));
                        box_height3.set((box_height3.get() - 20.0).max(50.0));
                        counter3.set(counter3.get() + 1);
                        log("Shrink: state updated");
                    },

                    Text("Shrink") {
                        fontSize: 16.0,
                        textColor: (255, 255, 255, 255),
                    }
                }

                // Change Color Button
                View {
                    width: 100.0,
                    height: 50.0,
                    color: (140u32, 60, 180, 255),
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    onTap: move |_| {
                        log("Color button clicked!");
                        let colors = [
                            (60u32, 140, 240, 255),
                            (240u32, 140, 60, 255),
                            (140u32, 240, 60, 255),
                            (240u32, 60, 140, 255),
                            (60u32, 240, 140, 255),
                        ];
                        let current = bg_color.get();
                        let idx = colors.iter().position(|&c| c == current).unwrap_or(0);
                        let next_idx = (idx + 1) % colors.len();
                        log(&format!("Color: changing to index {}/{}", next_idx, colors.len()));
                        bg_color.set(colors[next_idx]);
                        counter4.set(counter4.get() + 1);
                        log("Color: state updated");
                    },

                    Text("Color") {
                        fontSize: 16.0,
                        textColor: (255, 255, 255, 255),
                    }
                }
            }

            // Instructions
            View {
                width: "90%",
                flexGrow: 1.0,
                color: (30u32, 30, 40, 255),
                borderRadius: 8.0,
                padding: (10.0, 10.0, 10.0, 10.0),
                flexDirection: FlexDirection::Column,
                alignItems: AlignItems::FlexStart,

                Text("Instructions:") {
                    fontSize: 16.0,
                    textColor: (180, 180, 200, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }
                Text("• Tap 'Grow' to increase box size") {
                    fontSize: 14.0,
                    textColor: (150, 150, 170, 255),
                }
                Text("• Tap 'Shrink' to decrease box size") {
                    fontSize: 14.0,
                    textColor: (150, 150, 170, 255),
                }
                Text("• Tap 'Color' to cycle colors") {
                    fontSize: 14.0,
                    textColor: (150, 150, 170, 255),
                }
                Text("") {
                    fontSize: 14.0,
                    textColor: (150, 150, 170, 255),
                }
                Text("If working correctly, the box above") {
                    fontSize: 14.0,
                    textColor: (200, 150, 150, 255),
                }
                Text("should resize and change color") {
                    fontSize: 14.0,
                    textColor: (200, 150, 150, 255),
                }
                Text("when you tap the buttons.") {
                    fontSize: 14.0,
                    textColor: (200, 150, 150, 255),
                }
            }
        }
    }
}
