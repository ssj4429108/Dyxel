// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::focus;
use dyxel_shared::TextState;
use crate::{Signal as FsSignal, SHARED_BUFFER};
use dyxel_shared::push_command;

/// Type alias to match the prompt's Signal<T> syntax
pub type Signal<T> = Box<dyn FsSignal<Item = T> + Unpin + 'static>;

pub struct TextInput {
    pub id: u32,
    pub value: Signal<TextState>,
    pub on_change: Box<dyn Fn(TextState)>,
}

impl TextInput {
    pub fn render(&self) {
        let _is_focused = focus::get_focused_id() == self.id;
        // 渲染逻辑将在 Task 5 实现
    }
    
    pub fn handle_tap(&self) {
        focus::request_focus(self.id);
        // 通过 FFI 通知 Host 弹出键盘
        // 注意：此处 push_command! 是项目中的宏，请确认为可用
        push_command!(SHARED_BUFFER, SetTextInputFocused, self.id, 1u8);
        push_command!(SHARED_BUFFER, ShowTextInputKeyboard);
    }
}
