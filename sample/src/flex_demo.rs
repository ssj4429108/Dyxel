// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Flex Components Demo
//!
//! Demonstrates the enhanced Column, Row, Spacer, and Divider components with RSX macro support.

use dyxel_app::prelude::*;

#[app]
pub fn FlexDemo() -> impl BaseView {
    // Counter states for interactive demo
    let count = use_state(|| 0u32);

    rsx! {
        Column {
            width: "100%",
            height: "100%",
            background: Color::rgb(15, 15, 25),
            mainAxisAlignment: MainAxisAlignment::Start,
            crossAxisAlignment: CrossAxisAlignment::Center,
            padding: Padding::all(20.0),

            // Title
            Text("Flex Components Demo") {
                fontSize: 24.0,
                textColor: (255u8, 255, 255, 255),
            }

            Divider {
                color: Color::rgba(100, 100, 120, 128),
                thickness: 2.0,
            }

            // ===== Column Example =====
            Text("Column (Vertical Layout)") {
                fontSize: 16.0,
                textColor: (200u8, 200, 200, 255),
            }

            Column {
                spacing: 10.0,
                padding: Padding::all(16.0),
                background: Color::rgb(40, 40, 60),
                cornerRadius: 12.0,
                crossAxisAlignment: CrossAxisAlignment::Center,
                width: "90%",

                View {
                    width: 100.0,
                    height: 40.0,
                    color: (100u32, 60, 180, 255),
                    borderRadius: 8.0,
                }
                View {
                    width: 100.0,
                    height: 40.0,
                    color: (60u32, 180, 120, 255),
                    borderRadius: 8.0,
                }
                View {
                    width: 100.0,
                    height: 40.0,
                    color: (240u32, 180, 60, 255),
                    borderRadius: 8.0,
                }
            }

            Divider {
                color: Color::rgba(100, 100, 120, 128),
                thickness: 2.0,
            }

            // ===== Row Example =====
            Text("Row (Horizontal Layout)") {
                fontSize: 16.0,
                textColor: (200u8, 200, 200, 255),
            }

            Row {
                spacing: 8.0,
                mainAxisAlignment: MainAxisAlignment::SpaceBetween,
                crossAxisAlignment: CrossAxisAlignment::Center,
                padding: Padding::symmetric(16.0, 12.0),
                background: Color::rgb(40, 50, 60),
                cornerRadius: 8.0,
                width: "90%",
                height: 60.0,

                // Minus button
                Row {
                    width: 80.0,
                    height: 40.0,
                    color: (200u32, 80, 80, 255),
                    cornerRadius: 8.0,
                    mainAxisAlignment: MainAxisAlignment::Center,
                    crossAxisAlignment: CrossAxisAlignment::Center,
                    onTap: {
                        let c = count.clone();
                        move |_| { c.set(c.get().saturating_sub(1)); }
                    },

                    Text("-") {
                        fontSize: 20.0,
                        textColor: (255u8, 255, 255, 255),
                    }
                }

                Text("{count:.0}") {
                    fontSize: 18.0,
                    textColor: (255u8, 255, 255, 255),
                }

                // Plus button
                Row {
                    width: 80.0,
                    height: 40.0,
                    color: (80u32, 200, 120, 255),
                    cornerRadius: 8.0,
                    mainAxisAlignment: MainAxisAlignment::Center,
                    crossAxisAlignment: CrossAxisAlignment::Center,
                    onTap: {
                        let c = count.clone();
                        move |_| { c.set(c.get() + 1); }
                    },

                    Text("+") {
                        fontSize: 20.0,
                        textColor: (255u8, 255, 255, 255),
                    }
                }
            }

            Divider {
                color: Color::rgba(100, 100, 120, 128),
                thickness: 2.0,
            }

            // ===== Spacer Example =====
            Text("Spacer (Flexible Space)") {
                fontSize: 16.0,
                textColor: (200u8, 200, 200, 255),
            }

            Row {
                padding: Padding::all(12.0),
                background: Color::rgb(50, 45, 55),
                cornerRadius: 8.0,
                width: "90%",
                crossAxisAlignment: CrossAxisAlignment::Center,

                Text("Left") {
                    fontSize: 14.0,
                    textColor: (200u8, 200, 200, 255),
                }

                Spacer {
                    flex: 1.0,
                }

                Text("Center") {
                    fontSize: 14.0,
                    textColor: (200u8, 200, 200, 255),
                }

                Spacer {
                    flex: 2.0,
                }

                Text("Right") {
                    fontSize: 14.0,
                    textColor: (200u8, 200, 200, 255),
                }
            }

            Divider {
                color: Color::rgba(100, 100, 120, 128),
                thickness: 2.0,
            }

            // ===== Cross Axis Alignment Demo =====
            Text("Cross Axis Alignment") {
                fontSize: 16.0,
                textColor: (200u8, 200, 200, 255),
            }

            Row {
                spacing: 8.0,
                mainAxisAlignment: MainAxisAlignment::Center,
                crossAxisAlignment: CrossAxisAlignment::Center,
                padding: Padding::all(16.0),
                background: Color::rgb(45, 50, 60),
                cornerRadius: 8.0,
                width: "90%",
                height: 80.0,

                View {
                    width: 40.0,
                    height: 40.0,
                    color: (180u32, 100, 100, 255),
                    borderRadius: 20.0,
                }
                View {
                    width: 60.0,
                    height: 30.0,
                    color: (100u32, 180, 100, 255),
                    borderRadius: 4.0,
                }
                View {
                    width: 30.0,
                    height: 50.0,
                    color: (100u32, 100, 180, 255),
                    borderRadius: 4.0,
                }
            }
        }
    }
}
