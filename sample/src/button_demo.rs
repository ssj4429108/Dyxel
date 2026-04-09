// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Button Component Demo
//!
//! Demonstrates all Button variants and sizes from the Azure Meridian Design System.

use dyxel_app::prelude::*;
use dyxel_shared::TextAlign;

#[app]
pub fn ButtonDemo() -> impl BaseView {
    // Counter for interactive demo
    let count = use_state(|| 0u32);
    let last_action = use_state(|| "None".to_string());

    rsx! {
        Column {
            width: "100%",
            height: "100%",
            background: Color::rgb(249, 249, 255),
            mainAxisAlignment: MainAxisAlignment::Start,
            crossAxisAlignment: CrossAxisAlignment::Center,
            padding: Padding::all(20.0),

            // Title
            Text("Button Components") {
                fontSize: 28.0,
                textColor: (24u8, 28, 35, 255),
            }

            Text("Azure Meridian Design System") {
                fontSize: 14.0,
                textColor: (113u8, 119, 134, 255),
            }

            Divider {
                color: Color::rgba(193, 198, 215, 128),
                thickness: 1.0,
            }

            // ===== Primary Buttons =====
            Text("Primary Buttons") {
                fontSize: 16.0,
                textColor: (0u8, 88, 188, 255),
            }

            Row {
                spacing: 12.0,
                mainAxisAlignment: MainAxisAlignment::Center,
                crossAxisAlignment: CrossAxisAlignment::Center,
                padding: Padding::all(16.0),
                background: Color::rgb(255, 255, 255),
                cornerRadius: 12.0,
                width: "90%",

                Button("Small") {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Small,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Primary Small".to_string()); }
                    },
                }

                Button("Medium") {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Medium,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Primary Medium".to_string()); }
                    },
                }

                Button("Large") {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Large,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Primary Large".to_string()); }
                    },
                }
            }

            // ===== Secondary Buttons =====
            Text("Secondary Buttons") {
                fontSize: 16.0,
                textColor: (0u8, 102, 135, 255),
            }

            Row {
                spacing: 12.0,
                mainAxisAlignment: MainAxisAlignment::Center,
                crossAxisAlignment: CrossAxisAlignment::Center,
                padding: Padding::all(16.0),
                background: Color::rgb(255, 255, 255),
                cornerRadius: 12.0,
                width: "90%",

                Button("Small") {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Small,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Secondary Small".to_string()); }
                    },
                }

                Button("Medium") {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Medium,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Secondary Medium".to_string()); }
                    },
                }

                Button("Large") {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Large,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Secondary Large".to_string()); }
                    },
                }
            }

            // ===== Outline Buttons =====
            Text("Outline Buttons") {
                fontSize: 16.0,
                textColor: (0u8, 88, 188, 255),
            }

            Row {
                spacing: 12.0,
                mainAxisAlignment: MainAxisAlignment::Center,
                crossAxisAlignment: CrossAxisAlignment::Center,
                padding: Padding::all(16.0),
                background: Color::rgb(255, 255, 255),
                cornerRadius: 12.0,
                width: "90%",

                Button("Small") {
                    variant: ButtonVariant::Outline,
                    size: ButtonSize::Small,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Outline Small".to_string()); }
                    },
                }

                Button("Medium") {
                    variant: ButtonVariant::Outline,
                    size: ButtonSize::Medium,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Outline Medium".to_string()); }
                    },
                }

                Button("Large") {
                    variant: ButtonVariant::Outline,
                    size: ButtonSize::Large,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Outline Large".to_string()); }
                    },
                }
            }

            // ===== Ghost Buttons =====
            Text("Ghost Buttons") {
                fontSize: 16.0,
                textColor: (0u8, 88, 188, 255),
            }

            Row {
                spacing: 12.0,
                mainAxisAlignment: MainAxisAlignment::Center,
                crossAxisAlignment: CrossAxisAlignment::Center,
                padding: Padding::all(16.0),
                background: Color::rgb(241, 243, 254),
                cornerRadius: 12.0,
                width: "90%",

                Button("Small") {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Small,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Ghost Small".to_string()); }
                    },
                }

                Button("Medium") {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Medium,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Ghost Medium".to_string()); }
                    },
                }

                Button("Large") {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Large,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Ghost Large".to_string()); }
                    },
                }
            }

            // ===== Disabled State =====
            Text("Disabled State") {
                fontSize: 16.0,
                textColor: (113u8, 119, 134, 255),
            }

            Row {
                spacing: 12.0,
                mainAxisAlignment: MainAxisAlignment::Center,
                crossAxisAlignment: CrossAxisAlignment::Center,
                padding: Padding::all(16.0),
                background: Color::rgb(255, 255, 255),
                cornerRadius: 12.0,
                width: "90%",

                Button("Disabled") {
                    variant: ButtonVariant::Disabled,
                    size: ButtonSize::Medium,
                }

                Button("Primary") {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Medium,
                    disabled: true,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Should not trigger".to_string()); }
                    },
                }

                Button("Outline") {
                    variant: ButtonVariant::Outline,
                    size: ButtonSize::Medium,
                    disabled: true,
                }

                Button("Ghost") {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Medium,
                    disabled: true,
                }
            }

            // ===== Full Width Buttons =====
            Text("Full Width Buttons") {
                fontSize: 16.0,
                textColor: (0u8, 88, 188, 255),
            }

            Column {
                spacing: 12.0,
                padding: Padding::all(16.0),
                background: Color::rgb(255, 255, 255),
                cornerRadius: 12.0,
                width: "90%",
                crossAxisAlignment: CrossAxisAlignment::Stretch,

                Button("Primary Full Width") {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Medium,
                    expanded,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Full Width Primary".to_string()); }
                    },
                }

                Button("Secondary Full Width") {
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Medium,
                    expanded,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Full Width Secondary".to_string()); }
                    },
                }

                Button("Outline Full Width") {
                    variant: ButtonVariant::Outline,
                    size: ButtonSize::Medium,
                    expanded,
                    onTap: {
                        let action = last_action.clone();
                        move |_| { action.set("Full Width Outline".to_string()); }
                    },
                }
            }

            // ===== Action Feedback =====
            Text("Last Action") {
                fontSize: 14.0,
                textColor: (113u8, 119, 134, 255),
            }

            Row {
                width: "90%",
                height: 50.0,
                mainAxisAlignment: MainAxisAlignment::Center,
                crossAxisAlignment: CrossAxisAlignment::Center,
                background: Color::rgb(236, 237, 249),
                cornerRadius: 8.0,

                Text("{last_action}") {
                    fontSize: 16.0,
                    textColor: (24u8, 28, 35, 255),
                }
            }

            Spacer {
                flex: 1.0,
            }

            // ===== Counter Demo =====
            Text("Counter Demo") {
                fontSize: 16.0,
                textColor: (0u8, 88, 188, 255),
            }

            Row {
                spacing: 16.0,
                mainAxisAlignment: MainAxisAlignment::Center,
                crossAxisAlignment: CrossAxisAlignment::Center,
                padding: Padding::all(16.0),
                background: Color::rgb(255, 255, 255),
                cornerRadius: 12.0,
                width: "90%",

                Button("-") {
                    variant: ButtonVariant::Outline,
                    size: ButtonSize::Medium,
                    width: 60.0,
                    onTap: {
                        let c = count.clone();
                        let action = last_action.clone();
                        move |_| {
                            c.set(c.get().saturating_sub(1));
                            action.set("Decrement".to_string());
                        }
                    },
                }

                Text("{count:.0}") {
                    fontSize: 20.0,
                    textColor: (24u8, 28, 35, 255),
                    width: 60.0,
                    textAlign: TextAlign::Center,
                }

                Button("+") {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Medium,
                    width: 60.0,
                    onTap: {
                        let c = count.clone();
                        let action = last_action.clone();
                        move |_| {
                            c.set(c.get() + 1);
                            action.set("Increment".to_string());
                        }
                    },
                }
            }

            // Bottom spacing
            View {
                width: 1.0,
                height: 40.0,
            }
        }
    }
}
