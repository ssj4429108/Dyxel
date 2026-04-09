# TextInput Keyboard Events Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 Host → Guest 的键盘事件通道，实现 macOS 键盘输入与 TextInput 的完整集成

**Architecture:** 
1. 扩展 InputProxy 支持文本/键盘事件，复用现有的 RawInputEvent 缓冲区
2. macOS 层捕获键盘事件并通过 Host 接口传递到 InputProxy
3. WASM 端消费事件并更新 TextInput 状态，触发重渲染

**Tech Stack:** Rust, winit (macOS), dyxel-shared protocol

---

## File Structure

| File | Responsibility |
|------|----------------|
| `dyxel-shared/src/input.rs` | 扩展 RawInputEvent 支持键盘/文本事件类型 |
| `dyxel-core/src/input_proxy.rs` | 添加键盘事件处理方法 |
| `dyxel-core/src/lib.rs` | 暴露键盘事件处理公共接口 |
| `mac/src/main.rs` | 接入 winit 键盘事件 |
| `dyxel-view/src/lib.rs` | WASM 端消费输入事件并分发给 TextInput |
| `dyxel-view/src/components/text_input.rs` | 处理文本输入事件 |

---

## Task 1: 扩展共享输入事件协议

**Files:**
- Modify: `crates/dyxel-shared/src/input.rs`

**Context:** 当前 RawInputEvent 只支持 Pointer 事件，需要添加键盘和文本事件类型

- [ ] **Step 1: 添加键盘事件类型枚举**

在 `InputEventType` 中添加键盘相关事件：

```rust
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventType {
    PointerDown = 0,
    PointerMove = 1,
    PointerUp = 2,
    PointerCancel = 3,
    MouseWheel = 4,
    // === 新增键盘事件类型 ===
    KeyDown = 5,
    KeyUp = 6,
    TextInput = 7,        // 字符输入（已处理的文本）
    ImeComposition = 8,   // IME 合成中
    ImeCommit = 9,        // IME 提交
}
```

- [ ] **Step 2: 添加键盘事件数据结构**

在 input.rs 中添加：

```rust
/// 键盘事件数据（放在 RawInputEvent 的 padding 中复用）
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KeyEventData {
    /// 按键代码 (platform-specific)
    pub key_code: u32,
    /// 字符 UTF-32 编码（如果有）
    pub char_code: u32,
    /// 修饰键状态: bit0=shift, bit1=ctrl, bit2=alt, bit3=meta/cmd
    pub modifiers: u8,
    /// 重复按键计数
    pub repeat_count: u8,
    /// 预留
    pub _padding: [u8; 2],
}

/// 文本输入事件数据
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TextInputData {
    /// 文本长度（最大 16 字节内联存储）
    pub len: u8,
    /// 文本内容（UTF-8，内联存储）
    pub text: [u8; 16],
    /// 光标位置（在插入后）
    pub cursor_pos: u8,
    /// 预留
    pub _padding: [u8; 6],
}
```

- [ ] **Step 3: 扩展 RawInputEvent 支持联合类型**

修改 RawInputEvent，添加 union 风格的 payload：

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawInputEvent {
    pub timestamp: u64,
    pub pointer_id: u32,
    pub event_type: u8,
    pub _padding: [u8; 3],
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub target_node_id: u32,
    pub flags: u8,
    /// 事件类型特定的 payload
    pub payload: [u8; 23],
}

impl RawInputEvent {
    /// 获取键盘事件数据
    pub fn as_key_event(&self) -> Option<KeyEventData> {
        if matches!(self.event_type as u8, 5 | 6) { // KeyDown | KeyUp
            unsafe {
                Some(std::mem::transmute_copy(&self.payload))
            }
        } else {
            None
        }
    }
    
    /// 获取文本输入数据
    pub fn as_text_input(&self) -> Option<TextInputData> {
        if self.event_type == 7 { // TextInput
            unsafe {
                Some(std::mem::transmute_copy(&self.payload))
            }
        } else {
            None
        }
    }
}
```

- [ ] **Step 4: Commit**

```bash
git add crates/dyxel-shared/src/input.rs
git commit -m "feat: extend InputEventType with keyboard and text input events"
```

---

## Task 2: InputProxy 添加键盘事件支持

**Files:**
- Modify: `crates/dyxel-core/src/input_proxy.rs`

- [ ] **Step 1: 添加 NativeInputType 键盘变体**

```rust
pub enum NativeInputType {
    TouchDown,
    TouchMove,
    TouchUp,
    TouchCancel,
    MouseWheel { delta_x: f32, delta_y: f32 },
    PinchGesture { scale: f32 },
    RotationGesture { angle: f32 },
    // === 新增 ===
    KeyDown { key_code: u32, modifiers: u8 },
    KeyUp { key_code: u32, modifiers: u8 },
    TextInput { text: String },
}
```

- [ ] **Step 2: 添加键盘事件处理方法**

```rust
impl InputProxy {
    /// 处理键盘按下事件
    pub fn handle_key_down(
        &mut self,
        key_code: u32,
        modifiers: u8,
        shared_buffer: &mut SharedBuffer,
    ) {
        self.current_time = Self::current_time_micros();
        
        // 获取当前 focused 的 text input 节点
        let target_id = crate::text_input::focused_id();
        
        let event = RawInputEvent {
            timestamp: self.current_time,
            pointer_id: 0, // 键盘不使用 pointer_id
            event_type: InputEventType::KeyDown as u8,
            _padding: [0; 3],
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: target_id,
            flags: modifiers,
            payload: key_code.to_le_bytes().to_vec().try_into().unwrap_or([0; 23]),
        };
        
        self.push_event(shared_buffer, event);
    }
    
    /// 处理文本输入事件
    pub fn handle_text_input(
        &mut self,
        text: &str,
        shared_buffer: &mut SharedBuffer,
    ) {
        self.current_time = Self::current_time_micros();
        
        let target_id = crate::text_input::focused_id();
        if target_id == 0 {
            return; // 没有 focused 的输入框
        }
        
        // 截断到最大长度
        let text_bytes = text.as_bytes();
        let len = text_bytes.len().min(16);
        
        let mut payload = [0u8; 23];
        payload[0] = len as u8;
        payload[1..1+len].copy_from_slice(&text_bytes[..len]);
        
        let event = RawInputEvent {
            timestamp: self.current_time,
            pointer_id: 0,
            event_type: InputEventType::TextInput as u8,
            _padding: [0; 3],
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: target_id,
            flags: 0,
            payload,
        };
        
        self.push_event(shared_buffer, event);
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/dyxel-core/src/input_proxy.rs
git commit -m "feat: add keyboard event handling to InputProxy"
```

---

## Task 3: dyxel-core 暴露键盘事件公共接口

**Files:**
- Modify: `crates/dyxel-core/src/lib.rs`

- [ ] **Step 1: 添加公共导出函数**

在 `lib.rs` 的 `pub fn` 区域添加：

```rust
/// 处理键盘按下事件（由平台层调用）
pub fn handle_key_down(key_code: u32, modifiers: u8) {
    BRIDGE.with(|b| {
        if let Some(ref bridge) = *b.borrow() {
            let mut shared_buffer = bridge.lock_shared_buffer();
            if let Some(ref mut proxy) = *bridge.input_proxy.borrow_mut() {
                proxy.handle_key_down(key_code, modifiers, &mut shared_buffer);
            }
        }
    });
}

/// 处理键盘释放事件
pub fn handle_key_up(key_code: u32, modifiers: u8) {
    // 暂时与 key_down 类似，按需实现
}

/// 处理文本输入事件（由平台层调用）
pub fn handle_text_input(text: &str) {
    BRIDGE.with(|b| {
        if let Some(ref bridge) = *b.borrow() {
            let mut shared_buffer = bridge.lock_shared_buffer();
            if let Some(ref mut proxy) = *bridge.input_proxy.borrow_mut() {
                proxy.handle_text_input(text, &mut shared_buffer);
            }
        }
    });
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/dyxel-core/src/lib.rs
git commit -m "feat: expose keyboard event handlers in dyxel-core public API"
```

---

## Task 4: macOS 层接入 winit 键盘事件

**Files:**
- Modify: `mac/src/main.rs`

- [ ] **Step 1: 添加键盘事件处理**

在 winit 事件循环中添加：

```rust
// 在 Event::WindowEvent 匹配中添加：
Event::WindowEvent { 
    event: WindowEvent::KeyboardInput { 
        event: KeyEvent {
            state: ElementState::Pressed,
            logical_key,
            physical_key,
            text,
            location,
            repeat,
            ..
        },
        .. 
    }, 
    .. 
} => {
    // 忽略重复按键（长按）
    if repeat {
        return;
    }
    
    // 检查是否有 focused text input
    let focused = dyxel_core::text_input::focused_id();
    if focused == 0 {
        // 没有 focused 输入框，检查全局快捷键
        match logical_key {
            winit::keyboard::Key::Character(c) if c == "p" || c == "P" => {
                host.toggle_perf_overlay();
            }
            // ... 其他全局快捷键
            _ => {}
        }
        return;
    }
    
    // 有 focused 输入框，发送文本输入事件
    if let Some(t) = text {
        dyxel_core::handle_text_input(&t);
    } else {
        // 特殊按键处理（如 Backspace, Enter 等）
        match logical_key {
            winit::keyboard::Key::Named(named) => {
                match named {
                    winit::keyboard::NamedKey::Backspace => {
                        dyxel_core::handle_text_input("\u{8}"); // Backspace char
                    }
                    winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                        dyxel_core::handle_text_input("\n");
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add mac/src/main.rs
git commit -m "feat(macOS): integrate winit keyboard events with TextInput"
```

---

## Task 5: WASM 端消费输入事件

**Files:**
- Modify: `crates/dyxel-view/src/lib.rs`

- [ ] **Step 1: 添加输入事件处理循环**

在 `dyxel_view_tick` 或专门的事件处理函数中：

```rust
/// 处理来自 Host 的输入事件
pub fn process_input_events() {
    use dyxel_shared::{InputEventType, RawInputEvent};
    
    unsafe {
        let shared_buffer = &*SHARED_BUFFER;
        
        // 处理所有待处理的输入事件
        while let Some(event) = shared_buffer.input_buffer.pop() {
            match event.event_type as u8 {
                5 => { // KeyDown
                    if let Some(key_data) = event.as_key_event() {
                        handle_key_down_event(key_data, event.target_node_id);
                    }
                }
                7 => { // TextInput
                    if let Some(text_data) = event.as_text_input() {
                        let len = text_data.len as usize;
                        if len > 0 && len <= 16 {
                            let text = String::from_utf8_lossy(&text_data.text[..len]);
                            handle_text_input_event(&text, event.target_node_id);
                        }
                    }
                }
                _ => {
                    // 其他事件类型（pointer 等）已有处理
                }
            }
        }
    }
}

/// 处理文本输入事件
fn handle_text_input_event(text: &str, target_node_id: u32) {
    if target_node_id == 0 {
        return;
    }
    
    // 查找对应的 TextInput 组件并更新
    // 这里需要通过某种机制找到组件实例
    // 暂时通过全局回调表
    TEXT_INPUT_HANDLERS.with(|handlers| {
        if let Some(handler) = handlers.borrow().get(&target_node_id) {
            // 构造 TextState 更新
            let new_state = TextState {
                text: text.to_string(),
                selection: Selection::default(),
            };
            handler(new_state);
        }
    });
}

/// 处理键盘按下事件
fn handle_key_down_event(key_data: dyxel_shared::KeyEventData, target_node_id: u32) {
    // 处理特殊按键（如方向键、删除键等）
    // 普通字符输入走 TextInput 事件
}
```

- [ ] **Step 2: 确保事件处理被调用**

确保 `process_input_events` 在每帧被调用（在 `dyxel_view_tick` 中）：

```rust
#[no_mangle]
pub extern "C" fn dyxel_view_tick() {
    // 现有代码...
    
    // 处理输入事件
    process_input_events();
    
    // 继续现有代码...
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/dyxel-view/src/lib.rs
git commit -m "feat(wasm): consume keyboard events from Host and update TextInput"
```

---

## Task 6: 验证 Placeholder 渲染

**Files:**
- Modify: `crates/dyxel-render-vello/src/lib.rs`
- Test: 运行 text_input_demo 验证

- [ ] **Step 1: 检查 placeholder 渲染逻辑**

确认渲染代码存在（应该在 `render_node_text` 中）：

```rust
// 在渲染 TextInput 时
if let Some(text_input) = text_input_states.get(&id) {
    // 1. 如果文本为空且不是 focus 状态，显示 placeholder
    if editor.text().is_empty() && !text_input.focused {
        if !text_input.placeholder.is_empty() {
            let mut placeholder_editor = dyxel_editor::Editor::new(node.font_size);
            placeholder_editor.set_text(&text_input.placeholder);
            placeholder_editor.set_text_color(Color::from_rgba8(102, 102, 102, 204));
            placeholder_editor.draw(scene, align_transform);
        }
    }
    // ... 其他渲染
}
```

- [ ] **Step 2: 运行 demo 验证**

```bash
./build_mac.sh
# 运行后点击 TextInput，检查 placeholder 是否在 focus 前显示
```

- [ ] **Step 3: Commit**

如果发现问题，修复后提交。

---

## Task 7: Focus 视觉反馈

**Files:**
- Modify: `crates/dyxel-render-vello/src/lib.rs`

- [ ] **Step 1: 添加 focus 边框渲染**

在 TextInput 渲染时添加边框效果：

```rust
// 在 render_node_text 中
if let Some(text_input) = text_input_states.get(&id) {
    // 渲染 focus 边框
    if text_input.focused {
        let border_width = 2.0;
        let border_color = Color::from_rgb8(0, 122, 255); // iOS 蓝色
        
        // 获取节点布局
        let layout = get_node_layout(id); // 需要实现或获取
        
        render_border(
            scene,
            transform,
            layout.x,
            layout.y,
            layout.width,
            layout.height,
            border_width,
            border_color,
            node.border_radius,
        );
    }
}
```

- [ ] **Step 2: 实现 render_border 辅助函数**

```rust
fn render_border(
    scene: &mut Scene,
    transform: Affine,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    border_width: f64,
    color: Color,
    border_radius: f64,
) {
    use vello::kurbo::{RoundedRect, Stroke};
    use vello::peniko::Fill;
    
    let rect = RoundedRect::new(
        x - border_width / 2.0,
        y - border_width / 2.0,
        x + width + border_width / 2.0,
        y + height + border_width / 2.0,
        border_radius,
    );
    
    scene.stroke(
        &Stroke::new(border_width),
        transform,
        color,
        None,
        &rect,
    );
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/dyxel-render-vello/src/lib.rs
git commit -m "feat(rendering): add focus border for TextInput"
```

---

## 集成测试步骤

完成所有任务后，运行完整测试：

```bash
# 1. 构建项目
./build_mac.sh

# 2. 运行 demo
./target/release/dyxel

# 3. 测试场景
# - 点击 TextInput，placeholder 消失
# - 输入文字，显示光标
# - 光标闪烁（约 500ms 周期）
# - 点击外部，focus 消失，边框消失
# - 重新点击，恢复 focus
```

---

## Self-Review Checklist

- [ ] 所有 InputEventType 在 Host 和 Guest 端一致
- [ ] RawInputEvent 内存布局保持 32-byte 对齐
- [ ] macOS 键盘事件正确传递给 focused TextInput
- [ ] Placeholder 在空文本且非 focus 时显示
- [ ] Focus 状态有视觉反馈（边框）
- [ ] 光标在 focus 时显示并闪烁
