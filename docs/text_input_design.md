# TextInput 组件设计方案

## 1. 架构概览

### 1.1 核心设计原则
- **Host 负责原生集成**: 软键盘弹出、IME 输入、剪贴板访问
- **Guest 负责编辑逻辑**: 光标移动、文本编辑、选择范围
- **FFI 协议通信**: 通过共享内存和 opcode 进行双向通信

### 1.2 系统架构图

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Host (Native)                                   │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │   Platform   │  │ Text Input   │  │   Editor     │  │   Render     │     │
│  │   Events     │──│   Manager    │──│   Instance   │──│   Backend    │     │
│  │  (iOS/macOS) │  │              │  │              │  │  (Vello)     │     │
│  └──────────────┘  └──────┬───────┘  └──────┬───────┘  └──────────────┘     │
│                           │                 │                                │
│                      ┌────▼────┐       ┌────▼────┐                          │
│                      │  IME    │       │ Cursor  │                          │
│                      │ Bridge  │       │ Blinker │                          │
│                      └─────────┘       └─────────┘                          │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼ FFI Protocol
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Guest (WASM)                                    │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │  TextInput   │  │   Gesture    │  │   Editor     │  │   Command    │     │
│  │  Component   │──│   Handler    │──│    State     │──│    Buffer    │     │
│  │   (RSX)      │  │              │  │              │  │              │     │
│  └──────────────┘  └──────┬───────┘  └──────┬───────┘  └──────────────┘     │
│                           │                 │                                │
│                      ┌────▼────┐       ┌────▼────┐                          │
│                      │ onLong  │       │ onText  │                          │
│                      │  Press  │       │ Change  │                          │
│                      └─────────┘       └─────────┘                          │
└─────────────────────────────────────────────────────────────────────────────┘
```

## 2. 数据模型

### 2.1 TextInput State (Shared)

```rust
#[repr(C)]
pub struct TextInputState {
    /// Node ID
    pub node_id: u32,
    /// Cursor position (UTF-8 byte index)
    pub cursor_pos: u32,
    /// Selection start (if != cursor_pos, text is selected)
    pub selection_start: u32,
    /// Text content length (for buffer sizing)
    pub text_len: u32,
    /// Input type flags
    pub flags: TextInputFlags,
    /// Visual state
    pub visual: TextInputVisualState,
}

bitflags! {
    pub struct TextInputFlags: u32 {
        /// 是否启用
        const ENABLED = 1 << 0;
        /// 是否只读
        const READONLY = 1 << 1;
        /// 密码输入（显示圆点）
        const PASSWORD = 1 << 2;
        /// 多行输入
        const MULTILINE = 1 << 3;
        /// 自动校正
        const AUTOCORRECT = 1 << 4;
        /// 自动大写
        const AUTOCAPITALIZE = 1 << 5;
        /// 当前是否有焦点
        const FOCUSED = 1 << 8;
        /// 是否有选中文本
        const HAS_SELECTION = 1 << 9;
    }
}

pub struct TextInputVisualState {
    /// 光标可见性（用于闪烁）
    pub cursor_visible: bool,
    /// 最后一次光标切换时间戳
    pub last_blink_time: u64,
    /// 水平滚动偏移（用于长文本）
    pub scroll_offset_x: f32,
    /// 垂直滚动偏移
    pub scroll_offset_y: f32,
}
```

### 2.2 Editor Integration

重用 `dyxel-editor` crate 中的 `Editor` 结构，添加以下扩展：

```rust
impl Editor {
    /// 设置选择范围（用于长按选择）
    pub fn set_selection(&mut self, start: usize, end: usize);
    
    /// 获取当前选择范围
    pub fn selection_range(&self) -> Option<(usize, usize)>;
    
    /// 选中单词（双击）
    pub fn select_word_at(&mut self, pos: usize);
    
    /// 复制当前选择到剪贴板
    pub fn copy_selection(&self) -> Option<String>;
    
    /// 粘贴（需要 Host 支持）
    pub fn paste(&mut self, text: &str);
    
    /// 获取光标像素位置
    pub fn cursor_position(&self) -> (f32, f32);
}
```

## 3. FFI 协议设计

### 3.1 新增 Opcodes (Guest → Host)

```rust
// === TextInput Operations (100-119) ===
[100] CreateTextInput(id: u32),           // 创建文本输入节点
[101] SetTextInputFocused(id: u32, focused: u8),  // 设置焦点状态
[102] SetTextInputText(id: u32, len: u32),        // 设置文本内容
[103] SetTextInputCursor(id: u32, pos: u32),      // 设置光标位置
[104] SetTextInputSelection(id: u32, start: u32, end: u32), // 设置选择范围
[105] SetTextInputType(id: u32, input_type: u8),  // 设置输入类型
[106] ShowKeyboard(),                             // 请求显示软键盘
[107] HideKeyboard(),                             // 请求隐藏软键盘
[108] CopyToClipboard(id: u32),                   // 复制到剪贴板
[109] CutToClipboard(id: u32),                    // 剪切到剪贴板
[110] RequestPasteFromClipboard(id: u32),         // 请求粘贴

// === TextInput Configuration ===
[111] SetTextInputPlaceholder(id: u32, len: u32), // 设置占位符
[112] SetTextInputMaxLength(id: u32, max_len: u32), // 最大长度限制
[113] SetTextInputReturnKeyType(id: u32, key_type: u8), // 返回键类型
```

### 3.2 新增 Opcodes (Host → Guest)

```rust
// === TextInput Events (120-129) ===
[120] TextInputTextChanged(id: u32, len: u32),    // 文本变化（IME 输入）
[121] TextInputSelectionChanged(id: u32, start: u32, end: u32), // 选择变化
[122] KeyboardHeightChanged(height: f32),         // 键盘高度变化
[123] PasteFromClipboard(id: u32, len: u32),      // 粘贴内容

// === Cursor Blink Sync ===
[124] CursorBlinkTick(id: u32, visible: u8),      // 光标闪烁同步
```

### 3.3 共享内存布局

```rust
/// TextInput 共享数据区（位于 SharedBuffer 扩展区域）
#[repr(C)]
pub struct TextInputSharedData {
    /// 活跃输入框数量
    pub active_count: u32,
    /// 当前聚焦的输入框 ID（0 = 无）
    pub focused_id: u32,
    /// 键盘状态
    pub keyboard_state: KeyboardState,
    /// 文本内容缓冲区（循环使用）
    pub text_buffer: [u8; 4096],
    /// 缓冲区写入位置
    pub text_buffer_pos: u32,
}

pub struct KeyboardState {
    /// 是否可见
    pub visible: bool,
    /// 键盘高度（逻辑像素）
    pub height: f32,
    /// 键盘动画时长
    pub animation_duration_ms: u32,
}
```

## 4. 组件 API 设计

### 4.1 RSX 用法

```rust
// 基础用法
TextInput {
    placeholder: "请输入用户名",
    text: username.signal(),
    onTextChange: |text| { username.set(text); },
    onSubmit: |text| { submit_form(text); },
}

// 高级配置
TextInput {
    placeholder: "搜索",
    text: search_text.signal(),
    textColor: (255, 255, 255, 255),
    fontSize: 16.0,
    maxLength: 100,
    
    // 输入类型
    inputType: InputType::Text,
    // inputType: InputType::Password,
    // inputType: InputType::Number,
    // inputType: InputType::Email,
    // inputType: InputType::Phone,
    
    // 键盘配置
    returnKeyType: ReturnKeyType::Search,
    // returnKeyType: ReturnKeyType::Done,
    // returnKeyType: ReturnKeyType::Next,
    // returnKeyType: ReturnKeyType::Send,
    
    // 自动校正
    autoCorrect: false,
    autoCapitalize: AutoCapitalize::None,
    // autoCapitalize: AutoCapitalize::Sentences,
    // autoCapitalize: AutoCapitalize::Words,
    // autoCapitalize: AutoCapitalize::AllCharacters,
    
    // 事件回调
    onFocus: || { println!("获得焦点"); },
    onBlur: || { println_("失去焦点"); },
    onTextChange: |text| { search_text.set(text); },
    onSelectionChange: |start, end| { println!("选择范围: {}-{}"", start, end); },
    onSubmit: |text| { perform_search(text); },
    
    // 样式
    width: 300.0,
    height: 44.0,
    backgroundColor: (240, 240, 240, 255),
    borderRadius: 8.0,
    padding: (12.0, 16.0, 12.0, 16.0), // 上右下左
}

// 多行文本
TextInput {
    multiline: true,
    placeholder: "请输入描述...",
    text: description.signal(),
    maxLines: 5,
    minLines: 3,
    onTextChange: |text| { description.set(text); },
}
```

### 4.2 Rust API

```rust
pub struct TextInput {
    pub id: u32,
}

impl TextInput {
    /// 创建新的文本输入框
    pub fn new() -> Self;
    
    /// 设置/绑定文本
    pub fn text(self, text: impl Into<Prop<String>>) -> Self;
    
    /// 设置占位符
    pub fn placeholder(self, text: impl Into<Prop<String>>) -> Self;
    
    /// 设置输入类型
    pub fn input_type(self, input_type: InputType) -> Self;
    
    /// 设置最大长度
    pub fn max_length(self, len: u32) -> Self;
    
    /// 设置是否多行
    pub fn multiline(self, multiline: bool) -> Self;
    
    /// 设置是否启用
    pub fn enabled(self, enabled: bool) -> Self;
    
    /// 设置只读
    pub fn read_only(self, read_only: bool) -> Self;
    
    /// 设置焦点
    pub fn focus(self);
    
    /// 失去焦点
    pub fn blur(self);
    
    /// 全选
    pub fn select_all(self);
    
    /// 清空
    pub fn clear(self);
    
    /// 设置选择范围
    pub fn set_selection(self, start: usize, end: usize);
    
    /// 插入文本（在光标位置）
    pub fn insert_text(self, text: &str);
    
    /// 设置光标位置
    pub fn set_cursor_position(self, pos: usize);
    
    // === 事件回调 ===
    
    /// 文本变化回调
    pub fn on_text_change<F>(self, handler: F) -> Self
    where F: FnMut(String) + 'static;
    
    /// 获得焦点回调
    pub fn on_focus<F>(self, handler: F) -> Self
    where F: FnMut() + 'static;
    
    /// 失去焦点回调
    pub fn on_blur<F>(self, handler: F) -> Self
    where F: FnMut() + 'static;
    
    /// 提交回调（用户点击键盘上的返回/搜索键）
    pub fn on_submit<F>(self, handler: F) -> Self
    where F: FnMut(String) + 'static;
    
    /// 选择变化回调
    pub fn on_selection_change<F>(self, handler: F) -> Self
    where F: FnMut(usize, usize) + 'static;
}

/// 输入类型枚举
pub enum InputType {
    Text,       // 普通文本
    Password,   // 密码（显示圆点）
    Number,     // 数字键盘
    Email,      // 邮箱键盘
    Phone,      // 电话键盘
    URL,        // URL 键盘
    Decimal,    // 带小数点的数字
    Multiline,  // 多行文本
}

/// 返回键类型
pub enum ReturnKeyType {
    Default,
    Go,
    Google,
    Join,
    Next,
    Route,
    Search,
    Send,
    Done,
    EmergencyCall,
    Continue,
}

/// 自动大写类型
pub enum AutoCapitalize {
    None,
    Sentences,  // 句子首字母
    Words,      // 单词首字母
    AllCharacters, // 全大写
}
```

## 5. 渲染设计

### 5.1 渲染层次

```
┌─────────────────────────────────────┐
│  6. Cursor (闪烁)                   │  <- 最顶层
├─────────────────────────────────────┤
│  5. Selection Highlight             │  <- 半透明蓝色背景
├─────────────────────────────────────┤
│  4. Input Text                      │  <- 用户输入的文本
├─────────────────────────────────────┤
│  3. Placeholder Text (gray)         │  <- 当 text 为空时显示
├─────────────────────────────────────┤
│  2. Background / Border             │  <- 背景色、边框、圆角
├─────────────────────────────────────┤
│  1. Base Layer                      │  <- 基础容器
└─────────────────────────────────────┘
```

### 5.2 光标渲染

```rust
/// 光标样式
pub struct CursorStyle {
    /// 光标颜色
    pub color: Color,
    /// 光标宽度
    pub width: f32,
    /// 是否闪烁
    pub blink: bool,
    /// 闪烁周期（毫秒）
    pub blink_period_ms: u64,
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self {
            color: Color::from_rgb8(0, 122, 255), // iOS 蓝色
            width: 2.0,
            blink: true,
            blink_period_ms: 500,
        }
    }
}
```

### 5.3 选择高亮渲染

```rust
/// 选择高亮样式
pub struct SelectionStyle {
    /// 背景色
    pub background_color: Color,
    /// 是否显示选择锚点（圆圈）
    pub show_handles: bool,
    /// 锚点颜色
    pub handle_color: Color,
    /// 锚点大小
    pub handle_size: f32,
}

impl Default for SelectionStyle {
    fn default() -> Self {
        Self {
            background_color: Color::from_rgba8(100, 150, 255, 128),
            show_handles: true,
            handle_color: Color::from_rgb8(0, 122, 255),
            handle_size: 12.0,
        }
    }
}
```

## 6. 交互设计

### 6.1 手势处理

```rust
/// TextInput 手势处理器
pub struct TextInputGestureHandler {
    node_id: u32,
    state: TextInputGestureState,
}

enum TextInputGestureState {
    /// 空闲状态
    Idle,
    /// 追踪点击（判断是单击、双击还是长按）
    TrackingClick { start_time: u64, start_x: f32, start_y: f32 },
    /// 拖拽选择
    DragSelecting { start_pos: usize, current_pos: usize },
    /// 拖拽光标（单指移动光标）
    DraggingCursor { cursor_side: CursorSide },
}

enum CursorSide {
    Start, // 选择开始位置
    End,   // 选择结束位置（光标）
}

impl TextInputGestureHandler {
    /// 处理手势事件
    pub fn handle_gesture(&mut self, event: &GestureEvent) {
        match (self.state, event.gesture_type, event.phase) {
            // 单击 - 放置光标
            (Idle, Tap, Ended) => {
                self.place_cursor_at(event.x, event.y);
                self.show_keyboard();
            }
            
            // 双击 - 选中单词
            (_, Tap, Ended) if event.tap_count == 2 => {
                self.select_word_at(event.x, event.y);
            }
            
            // 长按 - 显示上下文菜单
            (_, LongPress, Began) => {
                self.show_context_menu(event.x, event.y);
            }
            
            // 拖拽 - 扩展选择
            (DragSelecting { .. }, Pan, Changed) => {
                self.extend_selection_to(event.x, event.y);
            }
            
            _ => {}
        }
    }
    
    /// 显示上下文菜单
    fn show_context_menu(&self, x: f32, y: f32) {
        // 通过 Host 接口显示原生上下文菜单
        // 选项：全选 / 复制 / 粘贴 / 剪切
        host::show_text_context_menu(
            self.node_id,
            x, y,
            &[
                MenuItem::SelectAll,
                MenuItem::Copy,
                MenuItem::Paste,
                MenuItem::Cut,
            ]
        );
    }
}
```

### 6.2 键盘事件处理

```rust
/// 键盘事件处理
impl Editor {
    pub fn handle_key_event(&mut self, event: &KeyEvent) {
        if !event.pressed {
            return; // 只处理按下事件
        }
        
        let shift = event.modifiers.shift;
        let ctrl = event.modifiers.ctrl;
        let meta = event.modifiers.meta;
        let cmd = cfg!(target_os = "macos") && meta;
        let ctrl_or_cmd = if cfg!(target_os = "macos") { meta } else { ctrl };
        
        match event.key {
            // 字符输入
            Key::Character(c) => {
                if ctrl_or_cmd {
                    match c.to_ascii_lowercase() {
                        'a' if !shift => self.select_all(),
                        'a' if shift => self.collapse_selection(),
                        'c' => self.copy(),
                        'x' => self.cut(),
                        'v' => self.request_paste(),
                        'z' if shift || (cmd && shift) => self.redo(),
                        'z' => self.undo(),
                        _ => {}
                    }
                } else {
                    self.insert(&c.to_string());
                }
            }
            
            // 导航键
            Key::Left => {
                if ctrl_or_cmd {
                    if shift { self.select_word_left(); } else { self.move_word_left(); }
                } else if shift {
                    self.select_left();
                } else {
                    self.move_left();
                }
            }
            Key::Right => { /* 类似 Left */ }
            Key::Up => { if shift { self.select_up(); } else { self.move_up(); } }
            Key::Down => { if shift { self.select_down(); } else { self.move_down(); } }
            Key::Home => { self.move_to_line_start(); }
            Key::End => { self.move_to_line_end(); }
            
            // 编辑键
            Key::Backspace => { self.backspace(); }
            Key::Delete => { self.delete(); }
            Key::Enter => { 
                if self.is_multiline {
                    self.insert("\n");
                } else {
                    self.submit();
                }
            }
            Key::Tab => { self.insert("\t"); }
            
            _ => {}
        }
    }
}
```

## 7. 原生平台集成

### 7.1 iOS/macOS (UIKit/AppKit)

```swift
// iOS 示例：集成 UITextInput 协议
class DyxelTextInputView: UIView, UITextInput {
    var nodeId: UInt32 = 0
    var editorBridge: EditorBridge?
    
    // MARK: - UITextInput
    
    var text: String? {
        get { editorBridge?.getText() }
        set { editorBridge?.setText(newValue ?? "") }
    }
    
    var selectedTextRange: UITextRange? {
        get {
            let range = editorBridge?.getSelectionRange()
            return TextRange(start: range?.start ?? 0, end: range?.end ?? 0)
        }
        set {
            if let range = newValue as? TextRange {
                editorBridge?.setSelection(start: range.start, end: range.end)
            }
        }
    }
    
    // MARK: - Keyboard
    
    override var canBecomeFirstResponder: Bool { true }
    
    override func becomeFirstResponder() -> Bool {
        let success = super.becomeFirstResponder()
        if success {
            editorBridge?.notifyFocus()
        }
        return success
    }
    
    override func resignFirstResponder() -> Bool {
        let success = super.resignFirstResponder()
        if success {
            editorBridge?.notifyBlur()
        }
        return success
    }
    
    // MARK: - Context Menu
    
    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        switch action {
        case #selector(copy(_:)),
             #selector(cut(_:)),
             #selector(paste(_:)),
             #selector(selectAll(_:)):
            return true
        default:
            return false
        }
    }
}
```

### 7.2 Android

```kotlin
// Android 示例
class DyxelTextInput(context: Context) : AppCompatEditText(context) {
    var nodeId: Long = 0
    var editorBridge: EditorBridge? = null
    
    init {
        // 禁用默认编辑，使用自定义逻辑
        isFocusableInTouchMode = true
        isCursorVisible = false // 我们自行渲染光标
        background = null // 自行渲染背景
        
        // 监听文本变化
        addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                editorBridge?.notifyTextChanged(s?.toString() ?: "")
            }
            
            override fun afterTextChanged(s: Editable?) {}
        })
    }
    
    override fun onFocusChanged(focused: Boolean, direction: Int, previouslyFocusedRect: Rect?) {
        super.onFocusChanged(focused, direction, previouslyFocusedRect)
        if (focused) {
            editorBridge?.notifyFocus()
            showKeyboard()
        } else {
            editorBridge?.notifyBlur()
            hideKeyboard()
        }
    }
}
```

## 8. 实现阶段

### Phase 1: 基础文本输入 (MVP)
- [ ] 创建 `TextInput` 组件结构
- [ ] 实现 FFI 协议（Opcodes 100-110）
- [ ] 基础光标渲染（不闪烁）
- [ ] 物理键盘输入支持
- [ ] iOS/macOS 软键盘集成

### Phase 2: 光标与选择
- [ ] 光标闪烁动画
- [ ] 点击放置光标
- [ ] 拖拽选择文本
- [ ] 双击选词
- [ ] 选择高亮渲染

### Phase 3: 长按菜单
- [ ] 长按手势识别
- [ ] 上下文菜单（全选/复制/粘贴/剪切）
- [ ] 剪贴板集成
- [ ] 菜单定位

### Phase 4: 高级功能
- [ ] IME 支持（中文/日文/韩文输入）
- [ ] 键盘避免（自动滚动输入框）
- [ ] 密码输入模式
- [ ] 输入验证

### Phase 5: 优化
- [ ] 性能优化（大文本）
- [ ] 减少重绘
- [ ] 内存优化

## 9. 关键设计决策

### 9.1 光标闪烁同步

**方案 A: Host 驱动**
- Host 维护光标闪烁定时器
- 通过 opcode 124 (CursorBlinkTick) 同步到 Guest
- **优点**: 与系统光标同步，节省 Guest CPU
- **缺点**: 增加 FFI 流量

**方案 B: Guest 自主**
- Guest 维护独立的闪烁定时器
- Host 只在需要时请求显示/隐藏
- **优点**: 简单，减少 FFI 调用
- **缺点**: 可能与系统光标不同步

**决策**: 采用方案 B（Guest 自主），因为：
1. Guest 有 `requestAnimationFrame` 机制
2. 减少 FFI 开销
3. 跨平台行为一致

### 9.2 文本存储位置

**方案 A: 纯 Host 存储**
- 所有文本存储在 Host
- Guest 通过 opcode 读写
- **优点**: Host 可直接访问，无需同步
- **缺点**: FFI 开销大，频繁编辑性能差

**方案 B: 纯 Guest 存储**
- 所有文本存储在 Guest
- Host 需要时通过 opcode 获取
- **优点**: Guest 编辑快
- **缺点**: Host IME 集成复杂

**方案 C: 双写（推荐）**
- Guest 维护主要文本（用于编辑）
- Host 维护副本（用于 IME）
- 通过事件同步
- **优点**: 平衡性能和集成复杂度
- **缺点**: 需要处理同步冲突

**决策**: 采用方案 C（双写）

### 9.3 键盘避免策略

```rust
/// 键盘避免配置
pub struct KeyboardAvoidConfig {
    /// 输入框与键盘的最小间距
    pub min_padding: f32,
    /// 动画时长
    pub animation_duration_ms: u64,
    /// 动画曲线
    pub animation_curve: AnimationCurve,
}

/// 当键盘弹出时
fn on_keyboard_show(keyboard_height: f32, input_rect: Rect) {
    let screen_height = get_screen_height();
    let keyboard_top = screen_height - keyboard_height;
    
    if input_rect.bottom > keyboard_top - config.min_padding {
        let offset = input_rect.bottom - (keyboard_top - config.min_padding);
        animate_container_offset(-offset, config.animation_duration_ms);
    }
}
```

## 10. 文件变更清单

### 新增文件
```
crates/dyxel-view/src/components/text_input.rs      # TextInput 组件
crates/dyxel-view/src/components/text_input/
├── cursor.rs                                         # 光标管理
├── selection.rs                                      # 选择管理
├── keyboard.rs                                       # 键盘事件
└── context_menu.rs                                   # 上下文菜单

crates/dyxel-core/src/text_input/
├── mod.rs                                            # Host 端 TextInput 管理
├── manager.rs                                        # 生命周期管理
├── keyboard_ios.rs                                   # iOS 键盘集成
├── keyboard_android.rs                               # Android 键盘集成
└── clipboard.rs                                      # 剪贴板集成

mac/src/TextInputView.swift                           # macOS/iOS 原生视图
android/app/src/main/java/com/dyxel/TextInputView.kt  # Android 原生视图
```

### 修改文件
```
crates/dyxel-shared/src/protocol.rs                   # 新增 opcodes
crates/dyxel-shared/src/state.rs                      # 新增 TextInputState
crates/dyxel-view/src/lib.rs                          # 导出 TextInput
crates/dyxel-view/src/components/mod.rs               # 注册 TextInput
crates/dyxel-render-vello/src/lib.rs                  # 渲染光标和选择
crates/dyxel-core/src/runtime.rs                      # 处理 TextInput opcodes
crates/dyxel-core/src/bridge.rs                       # Host-Guest 桥接
```
