// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! 简化版 TextInput 测试 - 使用 View 模拟输入框样式

use dyxel_app::prelude::*;
use dyxel_shared::TextAlign;

#[app]
pub fn TextInputSimpleTest() -> impl BaseView {
    rsx! {
        Column {
            width: "100%",
            height: "100%",
            background: Color::rgb(249, 249, 255),
            mainAxisAlignment: MainAxisAlignment::Start,
            crossAxisAlignment: CrossAxisAlignment::Center,
            padding: Padding::all(20.0),

            // 标题
            Text("TextInput Test") {
                fontSize: 28.0,
                fontWeight: 700,
                textColor: (33, 37, 41, 255),
            }

            // 普通 View 作为输入框背景
            View {
                width: "100%",
                height: 50.0,
                color: (255, 255, 255, 255),
                borderRadius: 8.0,
                borderWidth: 1.0,
                borderColor: (200, 200, 200, 255),
                margin: (20.0, 0.0, 0.0, 0.0),

                Text("Placeholder text") {
                    fontSize: 16.0,
                    textColor: (150, 150, 150, 255),
                }
            }

            // 另一个输入框样式
            View {
                width: "100%",
                height: 50.0,
                color: (255, 255, 255, 255),
                borderRadius: 8.0,
                borderWidth: 2.0,
                borderColor: (0, 123, 255, 255),
                margin: (16.0, 0.0, 0.0, 0.0),

                Text("Email input") {
                    fontSize: 16.0,
                    textColor: (33, 37, 41, 255),
                }
            }

            // 密码输入框样式
            View {
                width: "100%",
                height: 50.0,
                color: (255, 255, 255, 255),
                borderRadius: 8.0,
                borderWidth: 1.0,
                borderColor: (200, 200, 200, 255),
                margin: (16.0, 0.0, 0.0, 0.0),

                Text("••••••") {
                    fontSize: 16.0,
                    textColor: (33, 37, 41, 255),
                }
            }

            // 说明文字
            Text("如果上面的白色框可见，说明渲染正常") {
                fontSize: 14.0,
                textColor: (108, 117, 125, 255),
                margin: (20.0, 0.0, 0.0, 0.0),
            }
        }
    }
}
