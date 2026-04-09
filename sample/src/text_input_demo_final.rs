// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TextInput & Text 综合演示
//!
//! 展示 TextInput 和 Text 组件的各种功能，包括：
//! - 文本输入框（TextInput）
//! - 纯文本显示（Text）
//! - 混合布局
//! - 样式设置

use dyxel_app::prelude::*;
use dyxel_view::{TextInput, TextRenderable};

#[app]
pub fn TextInputDemoFinal() -> impl BaseView {
    rsx! {
        Column {
            width: "100%",
            height: "100%",
            background: Color::rgb(249, 249, 255),
            mainAxisAlignment: MainAxisAlignment::Start,
            crossAxisAlignment: CrossAxisAlignment::Center,
            padding: Padding::all(20.0),
            spacing: 16.0,

            // 标题
            Text("Text & TextInput Demo") {
                fontSize: 28.0,
                textColor: (24u8, 28, 35, 255),
            }

            Divider {
                color: Color::rgba(193, 198, 215, 128),
                thickness: 1.0,
            }

            // 纯文本示例
            Text("Pure Text Component") {
                fontSize: 20.0,

                textColor: (0u8, 88, 188, 255),
            }

            Text("This is a simple text display") {
                fontSize: 16.0,
                textColor: (80u8, 80, 80, 255),
            }

            Divider {
                color: Color::rgba(193, 198, 215, 128),
                thickness: 1.0,
            }

            // TextInput 示例
            Text("TextInput Component") {
                fontSize: 20.0,
                textColor: (0u8, 88, 188, 255),
            }

            // 简单输入框（带 placeholder）
            View {
                width: 280.0,
                height: 44.0,
                color: (255u32, 255, 255, 255),
                borderRadius: 8.0,

                TextInput {
                    fontSize: 16.0,
                    placeholder: "please enter...",
                }
            }

            // 带样式的输入框（带 placeholder）
            View {
                width: 280.0,
                height: 44.0,
                color: (255u32, 245, 240, 255),
                borderRadius: 8.0,

                TextInput {
                    fontSize: 16.0,
                    textColor: (180u8, 80, 60, 255),
                    placeholder: "Styled placeholder",
                }
            }

            Divider {
                color: Color::rgba(193, 198, 215, 128),
                thickness: 1.0,
            }

            // 底部说明
            Text("All components working correctly!") {
                fontSize: 14.0,
                textColor: (100u8, 100, 100, 255),
            }
        }
    }
}
