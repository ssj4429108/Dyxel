// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TextInput 自动化测试模块
//!
//! 提供模拟用户操作的测试工具：
//! - 模拟点击获取焦点
//! - 模拟键盘输入
//! - 验证渲染状态

use super::*;
use dyxel_shared::{TextInputState, TextInputRenderState};

/// 测试场景构建器
pub struct TextInputTestScenario {
    pub node_id: u32,
    pub initial_state: TextInputState,
}

impl TextInputTestScenario {
    /// 创建新的测试场景
    pub fn new(node_id: u32) -> Self {
        Self {
            node_id,
            initial_state: TextInputState::default(),
        }
    }

    /// 设置初始文本
    pub fn with_text(mut self, text: &str) -> Self {
        self.initial_state.text = text.to_string();
        self
    }

    /// 设置 placeholder
    pub fn with_placeholder(mut self, placeholder: &str) -> Self {
        self.initial_state.placeholder = placeholder.to_string();
        self
    }

    /// 设置为 focus 状态
    pub fn focused(mut self) -> Self {
        self.initial_state.focused = true;
        self.initial_state.cursor_visible = true;
        self
    }

    /// 执行测试
    pub fn run<F>(self, test_fn: F)
    where
        F: FnOnce(&mut TextInputTestContext),
    {
        // 创建 TextInput
        create_text_input(self.node_id);

        // 设置初始状态
        if !self.initial_state.text.is_empty() {
            set_text(self.node_id, self.initial_state.text.clone());
        }
        if !self.initial_state.placeholder.is_empty() {
            set_placeholder(self.node_id, self.initial_state.placeholder.clone());
        }
        if self.initial_state.focused {
            set_focused(self.node_id, true);
        }

        // 创建测试上下文
        let mut ctx = TextInputTestContext {
            node_id: self.node_id,
        };

        // 执行测试
        test_fn(&mut ctx);

        // 清理
        TextInputManager::with(|m| m.remove(self.node_id));
    }
}

/// 测试上下文
pub struct TextInputTestContext {
    node_id: u32,
}

impl TextInputTestContext {
    /// 模拟点击（获取焦点）
    pub fn tap(&mut self) {
        set_focused(self.node_id, true);
    }

    /// 模拟失去焦点
    pub fn blur(&mut self) {
        set_focused(self.node_id, false);
    }

    /// 模拟文本输入
    pub fn type_text(&mut self, text: &str) {
        if let Some(state) = TextInputManager::with(|m| m.get_mut(self.node_id).cloned()) {
            let mut new_state = state;
            new_state.insert_text(text);
            set_text(self.node_id, new_state.text);
            set_cursor_position(self.node_id, new_state.cursor_pos as u32);
        }
    }

    /// 模拟按下 Backspace
    pub fn backspace(&mut self) {
        if let Some(state) = TextInputManager::with(|m| m.get_mut(self.node_id).cloned()) {
            let mut new_state = state;
            new_state.backspace();
            set_text(self.node_id, new_state.text);
            set_cursor_position(self.node_id, new_state.cursor_pos as u32);
        }
    }

    /// 验证当前状态
    pub fn assert_state(&self, expected: impl FnOnce(&TextInputState) -> bool) -> bool {
        if let Some(state) = get(self.node_id) {
            expected(&state)
        } else {
            false
        }
    }

    /// 验证文本内容
    pub fn assert_text(&self, expected: &str) -> bool {
        self.assert_state(|s| s.text == expected)
    }

    /// 验证 focus 状态
    pub fn assert_focused(&self, expected: bool) -> bool {
        self.assert_state(|s| s.focused == expected)
    }

    /// 验证光标位置
    pub fn assert_cursor_at(&self, pos: usize) -> bool {
        self.assert_state(|s| s.cursor_pos == pos)
    }

    /// 获取当前状态
    pub fn current_state(&self) -> Option<TextInputState> {
        get(self.node_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_input_lifecycle() {
        TextInputTestScenario::new(1)
            .with_placeholder("Enter text...")
            .run(|ctx| {
                // 初始状态：未 focus，显示 placeholder
                assert!(ctx.assert_focused(false));
                assert!(ctx.assert_text(""));

                // 点击获取焦点
                ctx.tap();
                assert!(ctx.assert_focused(true));

                // 输入文本
                ctx.type_text("Hello");
                assert!(ctx.assert_text("Hello"));
                assert!(ctx.assert_cursor_at(5));

                // 失去焦点
                ctx.blur();
                assert!(ctx.assert_focused(false));

                // 重新获取焦点
                ctx.tap();
                assert!(ctx.assert_focused(true));
            });
    }

    #[test]
    fn test_backspace() {
        TextInputTestScenario::new(2)
            .with_text("Hello")
            .focused()
            .run(|ctx| {
                assert!(ctx.assert_text("Hello"));

                // 模拟多次 backspace
                ctx.backspace();
                assert!(ctx.assert_text("Hell"));

                ctx.backspace();
                assert!(ctx.assert_text("Hel"));

                ctx.backspace();
                ctx.backspace();
                ctx.backspace();
                assert!(ctx.assert_text(""));
            });
    }

    #[test]
    fn test_render_state_sync() {
        TextInputTestScenario::new(3)
            .with_text("Test")
            .with_placeholder("Placeholder")
            .focused()
            .run(|ctx| {
                // 同步到渲染器
                sync_to_renderer();

                // 这里可以验证渲染状态
                // 实际项目中可以检查全局状态或 mock 渲染器
            });
    }
}
