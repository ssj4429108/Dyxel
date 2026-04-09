// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_app::prelude::*;
use dyxel_shared::TextState;

// Use Prop from dyxel_view
use dyxel_view::Prop;

#[app]
pub fn TextInputTestFinal() -> impl BaseView {
    // 核心状态：TextState 包含文本和选区
    let s1 = use_state(|| TextState::new("Hello Dyxel"));
    let s2 = use_state(|| TextState::new("Focus Test"));

    // 显示状态
    let text1 = use_state(|| s1.get().text.clone());
    let text2 = use_state(|| s2.get().text.clone());

    rsx! {
        Column {
            width: "100%",
            height: "100%",
            background: Color::rgb(240, 240, 255),
            padding: Padding::all(20.0),
            spacing: 20.0,
            mainAxisAlignment: MainAxisAlignment::Start,
            crossAxisAlignment: CrossAxisAlignment::Center,

            Text("TextInput Spec Demo") { fontSize: 24.0 }

            Column {
                spacing: 8.0,
                width: "90%",
                background: Color::rgb(255, 255, 255),
                TextInput {
                    width: "100%",
                    height: 44.0,
                    placeholder: "Enter text here...",
                    // 显式构建 Prop，绕过 State -> Prop 的自动转换缺失
                    value: Prop::Dynamic(Box::new(s1.sig())),
                    color: (255, 255, 255, 255),
                    onChange: {
                        let s = s1.clone();
                        let t = text1.clone();
                        move |val: TextState| {
                            t.set(val.text.clone());
                            s.set(val);
                        }
                    },
                }
                Text("Text Content A: {text1}") { fontSize: 12.0 }
            }

            Column {
                spacing: 8.0,
                width: "90%",
                background: Color::rgb(255, 255, 255),
                TextInput {
                    width: "100%",
                    height: 44.0,
                    placeholder: "Enter text here...",
                    value: Prop::Dynamic(Box::new(s2.sig())),
                    color: (255, 255, 255, 255),
                    onChange: {
                        let s = s2.clone();
                        let t = text2.clone();
                        move |val: TextState| {
                            t.set(val.text.clone());
                            s.set(val);
                        }
                    },
                }
                Text("Text Content B: {text2}") { fontSize: 12.0 }
            }

            Button("Clear All") {
                variant: ButtonVariant::Outline,
                onTap: {
                    let a = s1.clone();
                    let b = s2.clone();
                    let t1 = text1.clone();
                    let t2 = text2.clone();
                    move |_| {
                        a.set(TextState::new(""));
                        b.set(TextState::new(""));
                        t1.set("".to_string());
                        t2.set("".to_string());
                    }
                },
            }
        }
    }
}
