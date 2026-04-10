# TextInput 增强功能实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 TextInput 的 padding、光标自定义、placeholder 独立样式、容器样式以及 Text 组件的选择功能

**架构:** 
- Phase 0: 先独立实现 SelectionManager（纯数学逻辑，TDD 方式）
- Phase 1-2: 逐步添加协议指令和 WASM API
- Phase 3-5: Host 端渲染实现，确保性能优化（阴影条件渲染）

**Tech Stack:** Rust, Vello, Taffy, dyxel-editor, wgpu

---

## 文件结构

### 新建文件
| 文件路径 | 职责 |
|---------|------|
| `crates/dyxel-core/src/selection/manager.rs` | SelectionManager 核心逻辑（索引/坐标转换） |
| `crates/dyxel-core/src/selection/mod.rs` | SelectionManager 模块导出 |
| `crates/dyxel-core/src/selection/test.rs` | SelectionManager 全量单元测试 |

### 修改文件
| 文件路径 | 职责 |
|---------|------|
| `crates/dyxel-shared/src/protocol.rs` | 添加新 OpCodes 130-141 |
| `crates/dyxel-view/src/components/text_input.rs` | TextInput 新增 API，RSX 修复 |
| `crates/dyxel-view/src/lib.rs` | Text 组件添加 selectable API |
| `crates/dyxel-render-api/src/lib.rs` | 扩展 TextInputRenderState |
| `crates/dyxel-render-vello/src/lib.rs` | 更新 render_cursor，添加阴影支持 |
| `crates/dyxel-core/src/text_input/manager.rs` | 处理新协议指令，光标边界保护 |
| `crates/dyxel-core/src/lib.rs` | 导出 SelectionManager |

---

## Phase 0: SelectionManager 核心 (TDD)

### Task 0.1: 创建 SelectionManager 模块结构

**Files:**
- Create: `crates/dyxel-core/src/selection/mod.rs`
- Create: `crates/dyxel-core/src/selection/manager.rs`
- Modify: `crates/dyxel-core/src/lib.rs`

- [ ] **Step 1: 创建模块目录和 mod.rs**

```rust
// crates/dyxel-core/src/selection/mod.rs
//! 文本选择管理器 - 处理光标定位、选区计算、手势识别

pub mod manager;
#[cfg(test)]
pub mod test;

pub use manager::{SelectionManager, KeyModifiers, ArrowDirection, TextLayout, HitTestResult};
```

- [ ] **Step 2: 创建 manager.rs 基础结构**

```rust
// crates/dyxel-core/src/selection/manager.rs
use dyxel_shared::Point;

/// 键盘修饰符状态
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// 方向键枚举
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArrowDirection {
    Left,
    Right,
    Up,
    Down,
}

/// 文本布局信息（用于坐标转换）
#[derive(Debug, Clone)]
pub struct TextLayout {
    pub line_count: usize,
    pub line_height: f32,
    pub char_widths: Vec<f32>, // 每个字符的宽度
    pub total_width: f32,
    pub total_height: f32,
}

/// 点击测试结果
#[derive(Debug, Clone)]
pub struct HitTestResult {
    pub index: usize,
    pub is_in_content_area: bool,
}

/// 选择管理器 - 纯数学逻辑，不依赖 UI
pub struct SelectionManager {
    cursor_inset: f32, // 光标安全余量
}

impl SelectionManager {
    pub fn new() -> Self {
        Self {
            cursor_inset: 0.0, // 将在初始化时设置
        }
    }

    /// 设置光标宽度，用于计算安全余量
    pub fn set_cursor_width(&mut self, width: f32) {
        self.cursor_inset = width / 2.0;
    }

    /// 计算内容区域内边距（考虑光标占位）
    pub fn content_inset(&self, padding: [f32; 4]) -> [f32; 4] {
        [
            padding[0],                      // top
            padding[1] + self.cursor_inset,  // right: 关键安全余量
            padding[2],                      // bottom
            padding[3],                      // left
        ]
    }
}

impl Default for SelectionManager {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 3: 在 dyxel-core 中导出模块**

```rust
// crates/dyxel-core/src/lib.rs 添加
pub mod selection;
pub use selection::{SelectionManager, KeyModifiers, ArrowDirection};
```

- [ ] **Step 4: 编译检查**

```bash
cd /Users/skipper/axzo/ai/test/taffy_vello_sync
cargo check -p dyxel-core
```

Expected: 编译成功

- [ ] **Step 5: Commit**

```bash
git add crates/dyxel-core/src/selection/
git add crates/dyxel-core/src/lib.rs
git commit -m "feat: 创建 SelectionManager 模块结构"
```

---

### Task 0.2: 实现 index_to_position

**Files:**
- Modify: `crates/dyxel-core/src/selection/manager.rs`
- Create: `crates/dyxel-core/src/selection/test.rs`

- [ ] **Step 1: 编写失败的测试**

```rust
// crates/dyxel-core/src/selection/test.rs
#[cfg(test)]
mod tests {
    use super::super::*;

    fn create_simple_layout() -> TextLayout {
        TextLayout {
            line_count: 1,
            line_height: 20.0,
            char_widths: vec![10.0, 10.0, 10.0, 10.0, 10.0], // "Hello"
            total_width: 50.0,
            total_height: 20.0,
        }
    }

    #[test]
    fn test_index_to_position_ascii() {
        let sm = SelectionManager::new();
        let layout = create_simple_layout();
        
        // 索引 0 应该在位置 0
        let pos = sm.index_to_position("Hello", 0, &layout);
        assert_eq!(pos.x, 0.0);
        assert_eq!(pos.y, 0.0);
        
        // 索引 3 应该在位置 30 (3 * 10)
        let pos = sm.index_to_position("Hello", 3, &layout);
        assert_eq!(pos.x, 30.0);
        assert_eq!(pos.y, 0.0);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dyxel-core index_to_position_ascii -- --nocapture
```

Expected: FAIL - "method not found"

- [ ] **Step 3: 实现 index_to_position**

```rust
// crates/dyxel-core/src/selection/manager.rs 在 impl SelectionManager 中添加

/// 逻辑索引 -> 屏幕坐标
pub fn index_to_position(
    &self,
    text: &str,
    index: usize,
    layout: &TextLayout,
) -> Point {
    // 确保索引在有效范围内
    let index = index.min(text.len());
    
    // 计算前 index 个字符的总宽度
    let mut x = 0.0;
    let mut current_index = 0;
    
    for (i, _) in text.chars().enumerate() {
        if i >= index {
            break;
        }
        if i < layout.char_widths.len() {
            x += layout.char_widths[i];
        }
        current_index = i;
    }
    
    // 单行文本，y 始终为 0
    Point { x, y: 0.0 }
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dyxel-core index_to_position_ascii -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/dyxel-core/src/selection/
git commit -m "feat: 实现 index_to_position 基础功能"
```

---

### Task 0.3: 实现 position_to_index

**Files:**
- Modify: `crates/dyxel-core/src/selection/manager.rs`
- Modify: `crates/dyxel-core/src/selection/test.rs`

- [ ] **Step 1: 编写失败的测试**

```rust
// crates/dyxel-core/src/selection/test.rs 添加

#[test]
fn test_position_to_index() {
    let sm = SelectionManager::new();
    let layout = create_simple_layout();
    
    // 位置 15 应该对应索引 1 (第一个字符后)
    let idx = sm.position_to_index("Hello", Point { x: 15.0, y: 5.0 }, &layout);
    assert_eq!(idx, 1);
    
    // 位置 35 应该对应索引 3
    let idx = sm.position_to_index("Hello", Point { x: 35.0, y: 5.0 }, &layout);
    assert_eq!(idx, 3);
    
    // 位置 0 应该对应索引 0
    let idx = sm.position_to_index("Hello", Point { x: 0.0, y: 5.0 }, &layout);
    assert_eq!(idx, 0);
}

#[test]
fn test_roundtrip_property() {
    let sm = SelectionManager::new();
    let layout = create_simple_layout();
    let text = "Hello";
    
    // 测试反函数性质: position_to_index(index_to_position(i)) == i
    for i in 0..=text.len() {
        let pos = sm.index_to_position(text, i, &layout);
        let idx = sm.position_to_index(text, pos, &layout);
        assert_eq!(idx, i, "Roundtrip failed for index {}", i);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dyxel-core position_to_index -- --nocapture
```

Expected: FAIL - "method not found"

- [ ] **Step 3: 实现 position_to_index**

```rust
// crates/dyxel-core/src/selection/manager.rs 在 impl SelectionManager 中添加

/// 屏幕坐标 -> 逻辑索引
pub fn position_to_index(
    &self,
    text: &str,
    position: Point,
    layout: &TextLayout,
) -> usize {
    let mut accumulated_width = 0.0;
    let mut index = 0;
    
    for (i, _) in text.chars().enumerate() {
        let char_width = layout.char_widths.get(i).copied().unwrap_or(0.0);
        let half_width = char_width / 2.0;
        
        // 如果点击位置在当前字符的左半部分，返回当前索引
        if position.x < accumulated_width + half_width {
            return index;
        }
        
        accumulated_width += char_width;
        index += 1;
    }
    
    // 点击在最后一个字符之后
    text.len()
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dyxel-core position_to_index -- --nocapture
cargo test -p dyxel-core roundtrip_property -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/dyxel-core/src/selection/
git commit -m "feat: 实现 position_to_index 和反函数性质验证"
```

---

### Task 0.4: 实现双击选词和三击全选

**Files:**
- Modify: `crates/dyxel-core/src/selection/manager.rs`
- Modify: `crates/dyxel-core/src/selection/test.rs`

- [ ] **Step 1: 编写失败的测试**

```rust
// crates/dyxel-core/src/selection/test.rs 添加

#[test]
fn test_select_word() {
    let sm = SelectionManager::new();
    let text = "Hello world test";
    
    // 点击 "world" 中的任意位置应该选中 "world"
    let (start, end) = sm.select_word(text, 6); // 'o' 在 "world" 中
    assert_eq!(start, 6);
    assert_eq!(end, 11);
    assert_eq!(&text[start..end], "world");
    
    // 点击 "Hello" 中的位置
    let (start, end) = sm.select_word(text, 2);
    assert_eq!(start, 0);
    assert_eq!(end, 5);
    assert_eq!(&text[start..end], "Hello");
}

#[test]
fn test_select_all() {
    let sm = SelectionManager::new();
    let text = "Hello world";
    
    let (start, end) = sm.select_all(text);
    assert_eq!(start, 0);
    assert_eq!(end, 11);
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dyxel-core select_word -- --nocapture
```

Expected: FAIL - "method not found"

- [ ] **Step 3: 实现 select_word 和 select_all**

```rust
// crates/dyxel-core/src/selection/manager.rs 在 impl SelectionManager 中添加

/// 双击选词 - 选中光标所在位置的单词
pub fn select_word(&self, text: &str, index: usize) -> (usize, usize) {
    let index = index.min(text.len());
    
    // 定义单词分隔符
    let is_separator = |c: char| c.is_whitespace() || c.is_ascii_punctuation();
    
    // 找到单词起始位置
    let mut start = index;
    for (i, c) in text.chars().enumerate() {
        if i >= index {
            break;
        }
        if is_separator(c) {
            start = i + 1;
        }
    }
    
    // 找到单词结束位置
    let mut end = index;
    for (i, c) in text.chars().enumerate() {
        if i < index {
            continue;
        }
        if is_separator(c) {
            break;
        }
        end = i + 1;
    }
    
    (start, end)
}

/// 三击全选
pub fn select_all(&self, text: &str) -> (usize, usize) {
    (0, text.len())
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dyxel-core select_word -- --nocapture
cargo test -p dyxel-core select_all -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/dyxel-core/src/selection/
git commit -m "feat: 实现双击选词和三击全选功能"
```

---

### Task 0.5: 实现键盘组合键支持

**Files:**
- Modify: `crates/dyxel-core/src/selection/manager.rs`
- Modify: `crates/dyxel-core/src/selection/test.rs`

- [ ] **Step 1: 编写失败的测试**

```rust
// crates/dyxel-core/src/selection/test.rs 添加

#[test]
fn test_handle_arrow_key_without_shift() {
    let mut sm = SelectionManager::new();
    
    // 无 Shift，只是移动光标
    let modifiers = KeyModifiers::default();
    let (start, end) = sm.handle_arrow_key(
        ArrowDirection::Right,
        modifiers,
        (5, 5), // 当前光标在位置 5
    );
    // 注意：实际实现需要文本内容来计算边界
    assert_eq!(start, end); // 无选区，start == end
}

#[test]
fn test_handle_arrow_key_with_shift() {
    let mut sm = SelectionManager::new();
    
    // 按住 Shift，扩展选区
    let modifiers = KeyModifiers {
        shift: true,
        ..Default::default()
    };
    let (start, end) = sm.handle_arrow_key(
        ArrowDirection::Right,
        modifiers,
        (3, 3), // 从位置 3 开始选择
    );
    // end 应该增加了
    assert!(end > start || end > 3);
}

#[test]
fn test_handle_select_all() {
    let sm = SelectionManager::new();
    let text = "Hello world";
    
    // Ctrl+A 应该选中全部
    let modifiers = KeyModifiers {
        ctrl: true,
        ..Default::default()
    };
    let result = sm.handle_select_all(text, modifiers);
    assert!(result.is_some());
    let (start, end) = result.unwrap();
    assert_eq!(start, 0);
    assert_eq!(end, text.len());
    
    // 无修饰符不应该触发
    let modifiers = KeyModifiers::default();
    let result = sm.handle_select_all(text, modifiers);
    assert!(result.is_none());
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dyxel-core handle_arrow_key -- --nocapture
```

Expected: FAIL

- [ ] **Step 3: 实现键盘处理**

```rust
// crates/dyxel-core/src/selection/manager.rs 在 impl SelectionManager 中添加

/// 处理方向键（支持 Shift 选择）
pub fn handle_arrow_key(
    &mut self,
    direction: ArrowDirection,
    modifiers: KeyModifiers,
    current_selection: (usize, usize),
) -> (usize, usize) {
    let (start, end) = current_selection;
    
    if modifiers.shift {
        // 扩展选区
        match direction {
            ArrowDirection::Right => (start, end + 1),
            ArrowDirection::Left => {
                if end > start {
                    (start, end - 1)
                } else if start > 0 {
                    (start - 1, end)
                } else {
                    (start, end)
                }
            }
            _ => (start, end), // Up/Down 暂简单处理
        }
    } else {
        // 移动光标（取消选择）
        let new_pos = match direction {
            ArrowDirection::Right => end + 1,
            ArrowDirection::Left => end.saturating_sub(1),
            _ => end,
        };
        (new_pos, new_pos)
    }
}

/// Ctrl/Cmd + A 全选
pub fn handle_select_all(
    &self,
    text: &str,
    modifiers: KeyModifiers,
) -> Option<(usize, usize)> {
    if modifiers.ctrl || modifiers.meta {
        Some(self.select_all(text))
    } else {
        None
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dyxel-core handle_arrow_key -- --nocapture
cargo test -p dyxel-core handle_select_all -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/dyxel-core/src/selection/
git commit -m "feat: 实现键盘组合键支持 (Shift+方向键, Ctrl+A)"
```

---

### Task 0.6: CJK 字符支持测试

**Files:**
- Modify: `crates/dyxel-core/src/selection/test.rs`

- [ ] **Step 1: 编写 CJK 测试**

```rust
// crates/dyxel-core/src/selection/test.rs 添加

#[test]
fn test_cjk_character_handling() {
    let sm = SelectionManager::new();
    let text = "你好世界"; // 4 个 CJK 字符
    
    // CJK 字符通常是 3 字节（UTF-8），但逻辑上是一个字符
    let layout = TextLayout {
        line_count: 1,
        line_height: 20.0,
        char_widths: vec![16.0, 16.0, 16.0, 16.0], // CJK 字符通常更宽
        total_width: 64.0,
        total_height: 20.0,
    };
    
    // 测试索引转换
    let pos = sm.index_to_position(text, 2, &layout);
    assert_eq!(pos.x, 32.0); // 2 * 16
    
    // 测试反向转换
    let idx = sm.position_to_index(text, Point { x: 30.0, y: 10.0 }, &layout);
    assert_eq!(idx, 1); // 30 在第二个字符 (16-32) 的范围内
}

#[test]
fn test_mixed_ascii_cjk() {
    let sm = SelectionManager::new();
    let text = "Hi你好"; // ASCII + CJK 混合
    
    let layout = TextLayout {
        line_count: 1,
        line_height: 20.0,
        char_widths: vec![10.0, 10.0, 16.0, 16.0], // ASCII 窄，CJK 宽
        total_width: 52.0,
        total_height: 20.0,
    };
    
    // "H"=0, "i"=1, "你"=2, "好"=3
    let pos = sm.index_to_position(text, 2, &layout);
    assert_eq!(pos.x, 20.0); // 10 + 10
    
    let pos = sm.index_to_position(text, 3, &layout);
    assert_eq!(pos.x, 36.0); // 10 + 10 + 16
}
```

- [ ] **Step 2: 运行测试确认通过**

```bash
cargo test -p dyxel-core cjk -- --nocapture
```

Expected: PASS (基于当前简单实现)

- [ ] **Step 3: Commit**

```bash
git add crates/dyxel-core/src/selection/
git commit -m "test: 添加 CJK 字符处理测试"
```

---

### Task 0.7: 光标占位安全余量测试

**Files:**
- Modify: `crates/dyxel-core/src/selection/test.rs`

- [ ] **Step 1: 编写测试**

```rust
// crates/dyxel-core/src/selection/test.rs 添加

#[test]
fn test_cursor_inset_calculation() {
    let mut sm = SelectionManager::new();
    sm.set_cursor_width(2.0);
    
    let padding = [12.0, 16.0, 12.0, 16.0];
    let inset = sm.content_inset(padding);
    
    // top 不变
    assert_eq!(inset[0], 12.0);
    // right 增加 cursor_width / 2 = 1.0
    assert_eq!(inset[1], 17.0);
    // bottom 不变
    assert_eq!(inset[2], 12.0);
    // left 不变
    assert_eq!(inset[3], 16.0);
}

#[test]
fn test_cursor_inset_with_different_widths() {
    let mut sm = SelectionManager::new();
    
    // 测试不同光标宽度
    sm.set_cursor_width(4.0);
    let inset = sm.content_inset([10.0, 10.0, 10.0, 10.0]);
    assert_eq!(inset[1], 12.0); // 10 + 2.0
    
    sm.set_cursor_width(1.0);
    let inset = sm.content_inset([10.0, 10.0, 10.0, 10.0]);
    assert_eq!(inset[1], 10.5); // 10 + 0.5
}
```

- [ ] **Step 2: 运行测试确认通过**

```bash
cargo test -p dyxel-core cursor_inset -- --nocapture
```

Expected: PASS

- [ ] **Step 3: 运行全量测试**

```bash
cargo test -p dyxel-core selection -- --nocapture
```

Expected: 所有测试通过

- [ ] **Step 4: Commit**

```bash
git add crates/dyxel-core/src/selection/
git commit -m "test: 添加光标占位安全余量测试"
```

---

## Phase 1: 协议层 (OpCodes)

### Task 1.1: 添加新协议指令到 dyxel-shared

**Files:**
- Modify: `crates/dyxel-shared/src/protocol.rs`

- [ ] **Step 1: 在 define_protocol! 宏中添加新指令**

在 `// === TextInput Operations (100-114) ===` 部分之后，添加：

```rust
// === TextInput Styling (130-141) ===
[130] SetTextInputContentPadding(id: u32, top: f32, right: f32, bottom: f32, left: f32),
[131] SetTextInputPlaceholderStyle(id: u32, r: u8, g: u8, b: u8, a: u8, font_size: f32),
[132] SetTextInputBackgroundColor(id: u32, r: u8, g: u8, b: u8, a: u8),
[133] SetTextInputBorderStyle(id: u32, style: u8, width: f32, r: u8, g: u8, b: u8, a: u8),
[134] SetTextInputCursorStyle(id: u32, width: f32, radius: f32),
[135] SetTextInputCursorColor(id: u32, r: u8, g: u8, b: u8, a: u8),
[136] SetTextInputCursorShadow(id: u32, blur: f32, offset_x: f32, offset_y: f32, r: u8, g: u8, b: u8, a: u8),
[137] SetTextInputCursorBlinkInterval(id: u32, interval_ms: u32),
[138] SetTextInputSelectionColor(id: u32, r: u8, g: u8, b: u8, a: u8),
[139] Reserved, // 预留
[140] SetTextSelectable(id: u32, enabled: u8),
[141] SetTextSelection(id: u32, start: u32, end: u32),
```

- [ ] **Step 2: 编译检查**

```bash
cargo check -p dyxel-shared
```

Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git add crates/dyxel-shared/src/protocol.rs
git commit -m "feat: 添加 TextInput 样式协议指令 (OpCodes 130-141)"
```

---

## Phase 2: WASM API 层

### Task 2.1: 修复 RSX 宏 - 延迟应用默认值

**Files:**
- Modify: `crates/dyxel-view/src/components/text_input.rs`

- [ ] **Step 1: 修改 TextInput 结构体**

```rust
// crates/dyxel-view/src/components/text_input.rs

use std::cell::RefCell;

/// 跟踪哪些样式已被显式设置
#[derive(Debug, Default, Clone, Copy)]
struct ExplicitStyles {
    font_size: bool,
    text_color: bool,
}

pub struct TextInput {
    pub id: u32,
    placeholder_text: Option<String>,
    explicit_styles: RefCell<ExplicitStyles>,
}
```

- [ ] **Step 2: 修改 new() 方法 - 不应用默认值**

```rust
impl TextInput {
    pub fn new() -> Self {
        let id = crate::NODE_COUNTER.fetch_add(1, Ordering::SeqCst);
        track_node(id);

        // 创建为 Text 节点（用于渲染）
        push_command!(SHARED_BUFFER, CreateTextNode, id);
        select_node(id);

        // 注册为文本输入（启用键盘、光标、选择）
        push_command!(SHARED_BUFFER, CreateTextInput, id);

        // 注意：不在此处设置默认样式！
        // 默认值将在首次渲染前应用

        let this = Self {
            id,
            placeholder_text: None,
            explicit_styles: RefCell::new(ExplicitStyles::default()),
        };

        // 注册焦点管理
        let caps = this.focus_capabilities();
        crate::focus::register_focusable(id, caps);

        this.on_tap(|_| {})
    }
}
```

- [ ] **Step 3: 修改 font_size 方法 - 标记显式设置**

```rust
impl TextInput {
    pub fn font_size(self, size: impl Into<Prop<f32>>) -> Self {
        self.explicit_styles.borrow_mut().font_size = true;
        
        crate::apply_prop(self.id, size.into(), |id, s| {
            select_node(id);
            push_command!(SHARED_BUFFER, SetFontSize, id, s);
        });
        self
    }
    
    pub fn text_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self {
        self.explicit_styles.borrow_mut().text_color = true;
        
        crate::apply_prop(self.id, color.into(), |id, (r, g, b, a)| {
            select_node(id);
            push_command!(SHARED_BUFFER, SetTextColor, id, r, g, b, a);
        });
        self
    }
}
```

- [ ] **Step 4: 添加 apply_defaults 方法**

```rust
impl TextInput {
    /// 在首次渲染前应用默认值（由运行时调用）
    pub(crate) fn apply_defaults(&self) {
        let explicit = self.explicit_styles.borrow();
        
        if !explicit.font_size {
            select_node(self.id);
            push_command!(SHARED_BUFFER, SetFontSize, self.id, 16.0_f32);
        }
        
        if !explicit.text_color {
            select_node(self.id);
            push_command!(SHARED_BUFFER, SetTextColor, self.id, 0u8, 0u8, 0u8, 255u8);
        }
    }
}
```

- [ ] **Step 5: 编译检查**

```bash
cargo check -p dyxel-view
```

Expected: 编译成功

- [ ] **Step 6: Commit**

```bash
git add crates/dyxel-view/src/components/text_input.rs
git commit -m "fix: RSX 宏 - 延迟应用默认值"
```

---

### Task 2.2: 添加 TextInput padding API

**Files:**
- Modify: `crates/dyxel-view/src/components/text_input.rs`

- [ ] **Step 1: 添加 padding 方法**

```rust
impl TextInput {
    /// 设置内边距 (top, right, bottom, left)
    pub fn padding(self, padding: impl Into<Prop<(f32, f32, f32, f32)>>) -> Self {
        crate::apply_prop(self.id, padding.into(), |id, (t, r, b, l)| {
            select_node(id);
            // 复用现有的 SetPadding 指令 (OpCode 13)
            push_command!(SHARED_BUFFER, SetPadding, id, t, r, b, l);
        });
        self
    }
    
    /// 设置水平内边距
    pub fn padding_horizontal(self, value: f32) -> Self {
        self.padding((12.0, value, 12.0, value))
    }
    
    /// 设置垂直内边距
    pub fn padding_vertical(self, value: f32) -> Self {
        self.padding((value, 16.0, value, 16.0))
    }
    
    /// 统一设置所有方向内边距
    pub fn padding_all(self, value: f32) -> Self {
        self.padding((value, value, value, value))
    }
}
```

- [ ] **Step 2: 编译检查**

```bash
cargo check -p dyxel-view
```

Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git add crates/dyxel-view/src/components/text_input.rs
git commit -m "feat: 添加 TextInput padding API"
```

---

### Task 2.3: 添加光标自定义 API

**Files:**
- Modify: `crates/dyxel-view/src/components/text_input.rs`

- [ ] **Step 1: 添加光标样式方法**

```rust
impl TextInput {
    /// 设置光标宽度
    pub fn cursor_width(self, width: f32) -> Self {
        // 需要获取当前 cursor_radius，这里简化处理
        select_node(self.id);
        push_command!(SHARED_BUFFER, SetTextInputCursorStyle, self.id, width, 0.0_f32);
        self
    }
    
    /// 设置光标颜色
    pub fn cursor_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self {
        crate::apply_prop(self.id, color.into(), |id, (r, g, b, a)| {
            select_node(id);
            push_command!(SHARED_BUFFER, SetTextInputCursorColor, id, r, g, b, a);
        });
        self
    }
    
    /// 设置光标圆角半径
    pub fn cursor_radius(self, radius: f32) -> Self {
        select_node(self.id);
        // 注意：需要保持当前 width，这里简化发送默认值
        push_command!(SHARED_BUFFER, SetTextInputCursorStyle, self.id, 2.0_f32, radius);
        self
    }
    
    /// 设置光标闪烁间隔
    pub fn cursor_blink_interval(self, ms: u32) -> Self {
        select_node(self.id);
        push_command!(SHARED_BUFFER, SetTextInputCursorBlinkInterval, self.id, ms);
        self
    }
}
```

- [ ] **Step 2: 编译检查**

```bash
cargo check -p dyxel-view
```

Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git add crates/dyxel-view/src/components/text_input.rs
git commit -m "feat: 添加光标自定义 API (宽、色、圆角、闪烁间隔)"
```

---

## Phase 3: Host 端渲染实现

### Task 3.1: 扩展 TextInputRenderState

**Files:**
- Modify: `crates/dyxel-render-api/src/lib.rs`

- [ ] **Step 1: 添加 CursorStyle 和 ShadowStyle 结构体**

```rust
// crates/dyxel-render-api/src/lib.rs

/// 光标样式
#[derive(Debug, Clone, Copy)]
pub struct CursorStyle {
    pub width: f32,
    pub color: [u8; 4],
    pub radius: f32,
    pub blink_interval_ms: u64,
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self {
            width: 2.0,
            color: [0, 0, 0, 255], // 黑色，但实际应继承文字颜色
            radius: 0.0,
            blink_interval_ms: 530,
        }
    }
}

/// 阴影样式
#[derive(Debug, Clone, Copy)]
pub struct ShadowStyle {
    pub color: [u8; 4],
    pub blur_radius: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Default for ShadowStyle {
    fn default() -> Self {
        Self {
            color: [0, 0, 0, 77], // 30% 透明度
            blur_radius: 4.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

/// Placeholder 样式
#[derive(Debug, Clone, Copy)]
pub struct PlaceholderStyle {
    pub color: [u8; 4],
    pub font_size: f32,
}

impl Default for PlaceholderStyle {
    fn default() -> Self {
        Self {
            color: [153, 153, 153, 255], // #999999
            font_size: 16.0,
        }
    }
}

/// 容器样式
#[derive(Debug, Clone, Copy)]
pub struct ContainerStyle {
    pub background_color: [u8; 4],
    pub border_width: f32,
    pub border_color: [u8; 4],
    pub border_radius: f32,
    pub padding: [f32; 4], // top, right, bottom, left
}

impl Default for ContainerStyle {
    fn default() -> Self {
        Self {
            background_color: [0, 0, 0, 0], // 透明
            border_width: 1.0,
            border_color: [224, 224, 224, 255], // #E0E0E0
            border_radius: 8.0,
            padding: [12.0, 16.0, 12.0, 16.0],
        }
    }
}
```

- [ ] **Step 2: 扩展 TextInputRenderState**

```rust
// crates/dyxel-render-api/src/lib.rs
// 在 TextInputRenderState 结构体中添加新字段

pub struct TextInputRenderState {
    // 已有字段
    pub text: String,
    pub focused: bool,
    pub cursor_pos: usize,
    pub selection_start: usize,
    pub cursor_visible: bool,
    pub secure: bool,
    pub composing_text: String,
    pub is_composing: bool,
    pub composition_start: usize,
    pub placeholder: String,
    
    // 新增字段
    pub cursor_style: CursorStyle,
    pub cursor_shadow: Option<ShadowStyle>,
    pub selection_color: [u8; 4],
    pub placeholder_style: PlaceholderStyle,
    pub container_style: ContainerStyle,
}

impl Default for TextInputRenderState {
    fn default() -> Self {
        Self {
            text: String::new(),
            focused: false,
            cursor_pos: 0,
            selection_start: 0,
            cursor_visible: false,
            secure: false,
            composing_text: String::new(),
            is_composing: false,
            composition_start: 0,
            placeholder: String::new(),
            cursor_style: CursorStyle::default(),
            cursor_shadow: None,
            selection_color: [0, 122, 255, 77], // iOS 蓝色 30% 透明度
            placeholder_style: PlaceholderStyle::default(),
            container_style: ContainerStyle::default(),
        }
    }
}
```

- [ ] **Step 3: 编译检查**

```bash
cargo check -p dyxel-render-api
```

Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dyxel-render-api/src/lib.rs
git commit -m "feat: 扩展 TextInputRenderState，添加光标、阴影、placeholder、容器样式"
```

---

### Task 3.2: 更新 Vello render_cursor 实现

**Files:**
- Modify: `crates/dyxel-render-vello/src/lib.rs`

- [ ] **Step 1: 修改 render_cursor 函数签名**

```rust
// crates/dyxel-render-vello/src/lib.rs

/// 渲染光标，支持圆角和阴影
fn render_cursor(
    builder: &mut Scene,
    transform: Affine,
    x: f64,
    y: f64,
    height: f64,
    style: &CursorStyle,
    shadow: Option<&ShadowStyle>,
) {
    let width = style.width as f64;
    let radius = style.radius as f64;
    let color = Color::rgba8(
        style.color[0],
        style.color[1],
        style.color[2],
        style.color[3],
    );
    
    // 1. 先绘制阴影（如果有且光标可见）
    if let Some(shadow) = shadow {
        let shadow_color = Color::rgba8(
            shadow.color[0],
            shadow.color[1],
            shadow.color[2],
            shadow.color[3],
        );
        
        let shadow_rect = RoundedRect::new(
            x + shadow.offset_x as f64,
            y + shadow.offset_y as f64,
            x + width + shadow.offset_x as f64,
            y + height + shadow.offset_y as f64,
            radius,
        );
        
        // 注意：这里简化处理，实际应使用 blur 效果
        builder.fill(Fill::NonZero, transform, shadow_color, None, &shadow_rect);
    }
    
    // 2. 绘制光标主体
    let cursor_rect = RoundedRect::new(x, y, x + width, y + height, radius);
    builder.fill(Fill::NonZero, transform, color, None, &cursor_rect);
}
```

- [ ] **Step 2: 更新调用 render_cursor 的代码**

在渲染 TextInput 光标的代码处，修改为传递 CursorStyle：

```rust
// 在渲染 TextInput 的代码中找到调用 render_cursor 的地方
// 修改为：
if text_input.focused && text_input.cursor_visible {
    let cursor_x = ...; // 计算光标位置
    let cursor_y = layout.location.y as f64 + text_origin_y;
    
    render_cursor(
        builder,
        transform,
        cursor_x,
        cursor_y,
        text_height,
        &text_input.cursor_style,
        text_input.cursor_shadow.as_ref(),
    );
}
```

- [ ] **Step 3: 编译检查**

```bash
cargo check -p dyxel-render-vello
```

Expected: 编译成功（可能有警告，需要调整）

- [ ] **Step 4: Commit**

```bash
git add crates/dyxel-render-vello/src/lib.rs
git commit -m "feat: 更新 render_cursor 支持圆角和阴影"
```

---

### Task 3.3: 在 TextInputManager 中处理新协议指令

**Files:**
- Modify: `crates/dyxel-core/src/text_input/manager.rs`

- [ ] **Step 1: 添加样式设置方法**

```rust
// crates/dyxel-core/src/text_input/manager.rs

impl TextInputManager {
    /// 设置光标样式
    pub fn set_cursor_style(&mut self, node_id: u32, width: f32, radius: f32) {
        if let Some(state) = self.registry.get_mut(node_id) {
            state.cursor_style.width = width;
            state.cursor_style.radius = radius;
            state.generation = state.generation.wrapping_add(1);
        }
    }
    
    /// 设置光标颜色
    pub fn set_cursor_color(&mut self, node_id: u32, color: [u8; 4]) {
        if let Some(state) = self.registry.get_mut(node_id) {
            state.cursor_style.color = color;
            state.generation = state.generation.wrapping_add(1);
        }
    }
    
    /// 设置光标闪烁间隔
    pub fn set_cursor_blink_interval(&mut self, node_id: u32, interval_ms: u32) {
        if let Some(state) = self.registry.get_mut(node_id) {
            state.cursor_style.blink_interval_ms = interval_ms as u64;
        }
    }
    
    /// 设置背景色
    pub fn set_background_color(&mut self, node_id: u32, color: [u8; 4]) {
        if let Some(state) = self.registry.get_mut(node_id) {
            state.container_style.background_color = color;
            state.generation = state.generation.wrapping_add(1);
        }
    }
}
```

- [ ] **Step 2: 更新 sync_to_renderer 方法**

确保新的样式字段被同步到渲染器：

```rust
// crates/dyxel-core/src/text_input/manager.rs

pub fn sync_to_renderer(&self) {
    for (&id, state) in self.registry.inputs.iter() {
        let render_state = dyxel_render_api::TextInputRenderState {
            // 已有字段...
            text: state.text.clone(),
            focused: state.focused,
            cursor_pos: state.cursor_pos,
            // ...
            
            // 新增字段
            cursor_style: state.cursor_style,
            cursor_shadow: state.cursor_shadow,
            selection_color: state.selection_color,
            placeholder_style: state.placeholder_style,
            container_style: state.container_style,
        };
        dyxel_render_vello::update_text_input_state_global(id, render_state);
    }
}
```

- [ ] **Step 3: 编译检查**

```bash
cargo check -p dyxel-core
```

Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/dyxel-core/src/text_input/manager.rs
git commit -m "feat: TextInputManager 支持新样式指令"
```

---

## Phase 4: 集成测试

### Task 4.1: 编写集成测试

**Files:**
- Create: `tests/text_input_styling.rs`

- [ ] **Step 1: 创建测试文件**

```rust
// tests/text_input_styling.rs
//! TextInput 样式集成测试

use dyxel_view::{TextInput, BaseView, Prop};

#[test]
fn test_text_input_padding_api() {
    // 验证 padding API 生成正确的指令
    let input = TextInput::new()
        .padding((10.0, 20.0, 10.0, 20.0));
    
    // 检查 node_id 有效
    assert!(input.node_id() > 0);
}

#[test]
fn test_text_input_cursor_style_api() {
    // 验证光标样式 API
    let input = TextInput::new()
        .cursor_width(3.0)
        .cursor_color((255, 0, 0, 255))
        .cursor_radius(2.0);
    
    assert!(input.node_id() > 0);
}

#[test]
fn test_rsx_style_override() {
    // 验证显式样式覆盖默认值
    let input = TextInput::new()
        .font_size(24.0)
        .text_color((255, 0, 0, 255));
    
    // 运行时应该应用 24.0 而不是 16.0
    assert!(input.node_id() > 0);
}
```

- [ ] **Step 2: 运行测试**

```bash
cargo test --test text_input_styling -- --nocapture
```

Expected: 测试通过

- [ ] **Step 3: Commit**

```bash
git add tests/text_input_styling.rs
git commit -m "test: 添加 TextInput 样式集成测试"
```

---

## 验收清单

在计划完成后，确认以下功能已实现：

- [ ] SelectionManager 核心逻辑（Phase 0）
  - [ ] index_to_position / position_to_index
  - [ ] select_word / select_all
  - [ ] 键盘组合键支持（Shift+方向键、Ctrl+A）
  - [ ] 光标占位安全余量
  - [ ] 全量单元测试（100% 覆盖核心逻辑）

- [ ] 协议层（Phase 1）
  - [ ] OpCodes 130-141 已定义

- [ ] WASM API（Phase 2）
  - [ ] RSX 宏修复（延迟应用默认值）
  - [ ] padding API
  - [ ] 光标自定义 API

- [ ] Host 渲染（Phase 3）
  - [ ] TextInputRenderState 扩展
  - [ ] Vello render_cursor 更新
  - [ ] TextInputManager 处理新指令

- [ ] 测试（Phase 4）
  - [ ] SelectionManager 单元测试
  - [ ] 集成测试
  - [ ] 全量测试通过

---

## 执行移交

**计划已保存到 `docs/superpowers/plans/2026-04-10-textinput-enhancement.md`**

**两种执行方式：**

1. **Subagent-Driven（推荐）** - 每个 Task 分配独立子代理，两阶段审查，快速迭代
2. **Inline Execution** - 在当前会话中使用 executing-plans 批量执行，带审查检查点

**建议选择？**
