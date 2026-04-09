// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TextInput & Text Component Demo
//!
//! 展示 TextInput 输入框和 Text 文本组件的各种功能。

use dyxel_app::prelude::*;
use dyxel_view::TextInput;
use dyxel_shared::{InputType, ReturnKeyType, TextAlign};

#[app]
pub fn TextInputDemo() -> impl BaseView {
    // 文本输入状态
    let user_name = use_state(|| "".to_string());

    // 创建输入框辅助函数
    let username_input = TextInput::new()
        .placeholder("请输入用户名")
        .input_type(InputType::Text);

    let email_input = TextInput::new()
        .placeholder("example@email.com")
        .input_type(InputType::Email);

    let password_input = TextInput::new()
        .placeholder("请输入密码")
        .input_type(InputType::Password)
        .secure(true);

    let search_input = TextInput::new()
        .placeholder("搜索...")
        .input_type(InputType::Text)
        .return_key_type(ReturnKeyType::Search);

    let dynamic_input = TextInput::new()
        .placeholder("输入你的名字...")
        .input_type(InputType::Text)
        .on_text_change({
            let user_name = user_name.clone();
            move |text: String| { user_name.set(text); }
        });

    rsx! {
        Column {
            width: "100%",
            height: "100%",
            background: Color::rgb(249, 249, 255),
            mainAxisAlignment: MainAxisAlignment::Start,
            crossAxisAlignment: CrossAxisAlignment::Center,
            padding: Padding::all(20.0),

            // 标题
            Text("TextInput & Text Demo") {
                fontSize: 28.0,
                fontWeight: 700,
                textColor: (33, 37, 41, 255),
            }

            Text("输入框与文本组件示例") {
                fontSize: 14.0,
                textColor: (108, 117, 125, 255),
            }

            // 内容区域
            Column {
                width: "100%",
                spacing: 20.0,
                margin: (20.0, 0.0, 0.0, 0.0),

                // ===== 基础文本输入 =====
                Text("基础文本输入") {
                    fontSize: 14.0,
                    fontWeight: 600,
                    textColor: (73, 80, 87, 255),
                }

                username_input

                // ===== 邮箱输入 =====
                Text("邮箱输入") {
                    fontSize: 14.0,
                    fontWeight: 600,
                    textColor: (73, 80, 87, 255),
                }

                email_input

                // ===== 密码输入 =====
                Text("密码输入 (安全模式)") {
                    fontSize: 14.0,
                    fontWeight: 600,
                    textColor: (73, 80, 87, 255),
                }

                password_input

                // ===== 搜索框 =====
                Text("搜索框") {
                    fontSize: 14.0,
                    fontWeight: 600,
                    textColor: (73, 80, 87, 255),
                }

                search_input

                // ===== 文本样式展示 =====
                Text("Text 组件样式") {
                    fontSize: 14.0,
                    fontWeight: 600,
                    textColor: (73, 80, 87, 255),
                    margin: (20.0, 0.0, 0.0, 0.0),
                }

                Text("默认文本 (16px)") {
                    fontSize: 16.0,
                }

                Text("粗体文本 (Bold)") {
                    fontSize: 16.0,
                    fontWeight: 700,
                }

                Text("小字号文本") {
                    fontSize: 12.0,
                    textColor: (108, 117, 125, 255),
                }

                Text("蓝色文本") {
                    fontSize: 14.0,
                    textColor: (0, 123, 255, 255),
                }

                Text("红色警告文本") {
                    fontSize: 14.0,
                    textColor: (220, 53, 69, 255),
                }

                Text("居中对齐文本") {
                    fontSize: 14.0,
                    textAlign: TextAlign::Center,
                    width: "100%",
                }

                Text("右对齐文本") {
                    fontSize: 14.0,
                    textAlign: TextAlign::End,
                    width: "100%",
                }

                // ===== 状态绑定示例 =====
                Text("状态绑定示例") {
                    fontSize: 14.0,
                    fontWeight: 600,
                    textColor: (73, 80, 87, 255),
                    margin: (20.0, 0.0, 0.0, 0.0),
                }

                dynamic_input

                Row {
                    width: "100%",
                    padding: (16.0, 16.0, 16.0, 16.0),
                    background: Color::rgb(232, 245, 253),
                    borderRadius: 8.0,

                    Text("你好, ") {
                        fontSize: 18.0,
                        textColor: (33, 37, 41, 255),
                    }

                    Text(user_name.get().to_string()) {
                        fontSize: 18.0,
                        fontWeight: 600,
                        textColor: (0, 123, 255, 255),
                    }

                    Text("!") {
                        fontSize: 18.0,
                        textColor: (33, 37, 41, 255),
                    }
                }

                // ===== 表单提交 =====
                Button("提交表单") {
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Large,
                    width: "100%",
                    margin: (20.0, 0.0, 0.0, 0.0),
                    onTap: |_event| {
                        // 表单提交逻辑
                    }
                }
            }
        }
    }
}
