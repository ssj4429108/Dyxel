// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_app::prelude::*;
use dyxel_shared::TextState;
use dyxel_view::Prop;

#[app]
pub fn TextInputEmptyTest() -> impl BaseView {
    // Empty text state - should show placeholder
    let s1 = use_state(|| TextState::new(""));
    let text1 = use_state(|| "Empty - type here".to_string());

    rsx! {
        Column {
            width: "100%",
            height: "100%",
            background: Color::rgb(240, 240, 255),
            padding: Padding::all(20.0),
            spacing: 20.0,
            mainAxisAlignment: MainAxisAlignment::Start,
            crossAxisAlignment: CrossAxisAlignment::Center,

            Text("TextInput Placeholder Test") { fontSize: 24.0 }

            Column {
                spacing: 8.0,
                width: "90%",
                background: Color::rgb(255, 255, 255),
                TextInput {
                    width: "100%",
                    height: 44.0,
                    placeholder: "Enter text here...",
                    value: Prop::Dynamic(Box::new(s1.sig())),
                    color: (255, 255, 255, 255),
                    onChange: {
                        let s = s1.clone();
                        let t = text1.clone();
                        move |val: TextState| {
                            t.set(format!("Content: '{}'", val.text));
                            s.set(val);
                        }
                    },
                }
                Text("{text1}") { fontSize: 12.0 }
            }

            Button("Clear") {
                variant: ButtonVariant::Outline,
                onTap: {
                    let a = s1.clone();
                    let t = text1.clone();
                    move |_| {
                        a.set(TextState::new(""));
                        t.set("Empty - type here".to_string());
                    }
                },
            }
        }
    }
}
