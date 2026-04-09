# TextInput 响应式与焦点系统实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现一个基于 WASM 渲染、响应式 Signal 绑定且具备焦点抢占机制的 TextInput 组件。

**Architecture:** 采用混合事务架构。Native 负责 IME 输入并发送 TextUpdateTransaction，WASM 持有状态真相并通过 Vello 像素级渲染光标与选区。引入全局 FocusManager 处理多输入框的焦点抢占。

**Tech Stack:** Rust (WASM), Vello (Rendering), Taffy (Layout), Signal-based State (Reactive).

---

### Task 1: 定义核心数据模型与 TextState

**Files:**
- Create: `crates/dyxel-shared/src/text.rs`
- Modify: `crates/dyxel-shared/src/lib.rs`

- [ ] **Step 1: 创建 TextState 与 TextSelection 结构体**

```rust
// crates/dyxel-shared/src/text.rs
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextSelection {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextState {
    pub text: String,
    pub selection: TextSelection,
    pub composing: Option<(usize, usize)>,
}

impl TextState {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            selection: TextSelection { start: text.len(), end: text.len() },
            composing: None,
        }
    }
}
```

- [ ] **Step 2: 在共享库中导出模型**

```rust
// crates/dyxel-shared/src/lib.rs
pub mod text;
pub use text::{TextState, TextSelection};
```

- [ ] **Step 3: 提交变更**

```bash
git add crates/dyxel-shared/src/text.rs crates/dyxel-shared/src/lib.rs
git commit -m "feat(shared): add TextState and TextSelection models"
```

---

### Task 2: 扩展 FFI 协议 (OpCodes)

**Files:**
- Modify: `crates/dyxel-shared/src/protocol.rs`

- [ ] **Step 1: 新增 TextInput 相关 OpCodes**

```rust
// crates/dyxel-shared/src/protocol.rs
pub enum OpCode {
    // ... 现有代码
    SetTextInputFocused = 101, // (id: u32, focused: u8, keyboard_type: u8)
    SyncTextState = 102,       // (id: u32, text_len: u32, selection_start: u32, selection_end: u32, ...)
    OnTextUpdate = 120,        // (id: u32, text_len: u32, selection_start: u32, selection_end: u32)
    OnAction = 121,            // (id: u32, action: u8)
}
```

- [ ] **Step 2: 提交变更**

```bash
git add crates/dyxel-shared/src/protocol.rs
git commit -m "feat(shared): add TextInput Opcodes for FFI sync"
```

---

### Task 3: 实现全局 FocusManager (WASM)

**Files:**
- Create: `crates/dyxel-view/src/focus.rs`
- Modify: `crates/dyxel-view/src/lib.rs`

- [ ] **Step 1: 创建 FocusManager 结构**

```rust
// crates/dyxel-view/src/focus.rs
use std::sync::atomic::{AtomicU32, Ordering};

static FOCUSED_ID: AtomicU32 = AtomicU32::new(0);

pub fn request_focus(id: u32) {
    let prev = FOCUSED_ID.swap(id, Ordering::SeqCst);
    if prev != 0 && prev != id {
        // TODO: 触发旧节点的 blur 回调 (在后续组件实现中补充)
        log::debug!("Focus preempted: {} -> {}", prev, id);
    }
}

pub fn get_focused_id() -> u32 {
    FOCUSED_ID.load(Ordering::SeqCst)
}

pub fn clear_focus() {
    FOCUSED_ID.store(0, Ordering::SeqCst);
}
```

- [ ] **Step 2: 导出焦点模块**

```rust
// crates/dyxel-view/src/lib.rs
pub mod focus;
```

- [ ] **Step 3: 提交变更**

```bash
git add crates/dyxel-view/src/focus.rs crates/dyxel-view/src/lib.rs
git commit -m "feat(view): add global FocusManager for focus preemption"
```

---

### Task 4: 创建 TextInput 组件外壳与响应式绑定

**Files:**
- Create: `crates/dyxel-view/src/components/text_input_new.rs`

- [ ] **Step 1: 定义组件结构与 Signal 绑定**

```rust
// crates/dyxel-view/src/components/text_input_new.rs
use crate::focus;
use dyxel_shared::TextState;

pub struct TextInput {
    pub id: u32,
    pub value: Signal<TextState>,
    pub on_change: Box<dyn Fn(TextState)>,
}

impl TextInput {
    pub fn render(&self) {
        let is_focused = focus::get_focused_id() == self.id;
        // 渲染逻辑将在 Task 5 实现
    }
    
    pub fn handle_tap(&self) {
        focus::request_focus(self.id);
        // 通过 FFI 通知 Host 弹出键盘
        push_command!(SHARED_BUFFER, SetTextInputFocused, self.id, 1u8, 0u8);
    }
}
```

- [ ] **Step 2: 提交变更**

```bash
git add crates/dyxel-view/src/components/text_input_new.rs
git commit -m "feat(view): implement TextInput component shell with signal binding"
```

---

### Task 5: Vello 光标与选区渲染实现

**Files:**
- Modify: `crates/dyxel-render-vello/src/lib.rs`

- [ ] **Step 1: 实现光标渲染逻辑**

```rust
// crates/dyxel-render-vello/src/lib.rs
fn render_cursor(builder: &mut SceneBuilder, x: f32, y: f32, height: f32, color: Color) {
    let rect = Rect::new(x, y, x + 2.0, y + height);
    builder.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
}
```

- [ ] **Step 2: 实现选区高亮逻辑**

```rust
// crates/dyxel-render-vello/src/lib.rs
fn render_selection(builder: &mut SceneBuilder, rects: &[Rect], color: Color) {
    for rect in rects {
        builder.fill(Fill::NonZero, Affine::IDENTITY, color, None, rect);
    }
}
```

- [ ] **Step 3: 在渲染循环中集成组件渲染**

```rust
// crates/dyxel-render-vello/src/lib.rs 内部循环
if node.is_text_input() && node.focused {
    let cursor_pos = calculate_cursor_pos(node);
    render_cursor(builder, cursor_pos.x, cursor_pos.y, node.line_height, CURSOR_COLOR);
}
```

- [ ] **Step 4: 提交变更**

```bash
git add crates/dyxel-render-vello/src/lib.rs
git commit -m "feat(render): implement Vello cursor and selection rendering"
```

---

### Task 6: Host 端事务调和 (Reconciliation)

**Files:**
- Modify: `crates/dyxel-core/src/text_input/manager.rs`

- [ ] **Step 1: 处理来自 WASM 的 SyncTextState 指令**

```rust
// crates/dyxel-core/src/text_input/manager.rs
fn handle_sync_text_state(&mut self, id: u32, state: TextState) {
    if let Some(native_input) = self.native_inputs.get_mut(&id) {
        native_input.update_state(state);
    }
}
```

- [ ] **Step 2: 将 Native 变更包装为 OnTextUpdate 事务**

```rust
// crates/dyxel-core/src/text_input/manager.rs
fn on_native_input_changed(&mut self, id: u32, new_text: String) {
    // 发送事务给 WASM
    self.bridge.send_event(OpCode::OnTextUpdate, id, new_text);
}
```

- [ ] **Step 3: 提交变更**

```bash
git add crates/dyxel-core/src/text_input/manager.rs
git commit -m "feat(core): implement Host-side text transaction reconciliation"
```
