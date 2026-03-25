# Editor 集成设计文档

基于 vello_editor 的完整文本编辑方案

## 架构概览

```
┌─────────────────────────────────────────────────────────────┐
│                      dyxel-editor                           │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │ FontContext  │  │LayoutContext │  │ PlainEditor  │      │
│  │  (字体缓存)   │  │  (布局缓存)   │  │ (文本/光标/选区)│     │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   dyxel-render-vello                        │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              VelloBackend                           │   │
│  │  ┌─────────────────────────────────────────────┐   │   │
│  │  │  editors: HashMap<u32, Editor>              │   │   │
│  │  │  - 每个 Text 节点对应一个 Editor             │   │   │
│  │  │  - 自动创建/更新/删除                        │   │   │
│  │  └─────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## 核心组件

### 1. Editor (dyxel-editor/src/lib.rs)

封装 Parley 的 PlainEditor，提供高级 API：

```rust
pub struct Editor {
    font_cx: FontContext,           // 字体数据库
    layout_cx: LayoutContext<Brush>, // 布局上下文
    editor: PlainEditor<Brush>,     // 核心编辑器
    cursor_visible: bool,
}
```

#### 主要 API

**文本操作：**
- `set_text(&mut self, text: &str)` - 设置文本
- `insert(&mut self, text: &str)` - 插入文本
- `delete(&mut self)` / `backspace(&mut self)` - 删除

**光标移动：**
- `move_left/right/up/down(&mut self)` - 方向移动
- `move_to_line_start/end(&mut self)` - 行首/行尾
- `move_to_text_start/end(&mut self)` - 文本起止
- `move_to_point(&mut self, x: f32, y: f32)` - 鼠标点击定位

**选区操作：**
- `select_all(&mut self)` - 全选
- `select_left/right/up/down(&mut self)` - 扩展选区
- `select_word_at_point(&mut self, x, y)` - 双击选词
- `extend_selection_to_point(&mut self, x, y)` - 拖拽选区
- `collapse_selection(&mut self)` - 取消选择

**渲染：**
- `draw(&mut self, scene: &mut Scene, transform: Affine) -> Generation`

### 2. 输入处理 (dyxel-editor/src/input.rs)

```rust
pub fn handle_keyboard(&mut self, event: &KeyboardEvent)
pub fn handle_pointer(&mut self, event: &PointerEvent)
pub fn handle_double_click(&mut self, x: f32, y: f32)
pub fn handle_drag(&mut self, x: f32, y: f32)
```

## 渲染流程

### 1. 布局阶段

```rust
// render_internal 中

// 1. 为 Text 节点创建/更新 Editor
for (&id, node) in &g.nodes {
    if node.view_type == ViewType::Text {
        let editor = editors.entry(id).or_insert_with(|| {
            let mut ed = Editor::new(node.font_size);
            ed.set_text(&node.text);
            ed.set_text_color(node.color);
            ed
        });
    }
}

// 2. 使用 Editor 测量文本尺寸进行 Taffy 布局
let _ = g.taffy.compute_layout_with_measure(rn, ..., |...| {
    if let Some(editor) = editors.get_mut(&editor_id) {
        editor.set_width(Some(width));
        let (w, h) = editor.layout_size();
        return Size { width: w, height: h };
    }
});
```

### 2. 渲染阶段

```rust
fn render_node_recursive_with_transform(...) {
    if node.view_type == ViewType::Text {
        if let Some(editor) = editors.get_mut(&id) {
            editor.set_width(Some(layout.size.width));
            let text_transform = transform * Affine::translate((x, y));
            editor.draw(scene, text_transform);
        }
    }
}
```

### Editor::draw 内部

1. **绘制选区背景** - 蓝色半透明矩形
2. **绘制光标** - 白色竖线（带闪烁）
3. **绘制文字** - 使用 `scene.draw_glyphs()`

```rust
for line in layout.lines() {
    for item in line.items() {
        let PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
        
        scene
            .draw_glyphs(font)
            .brush(&style.brush)
            .hint(true)
            .transform(transform)
            .font_size(font_size)
            .glyph_transform(glyph_xform)
            .normalized_coords(run.normalized_coords())
            .draw(Fill::NonZero, glyphs);
    }
}
```

## 脏检查与优化

```rust
// Generation 用于脏检查
pub fn generation(&self) -> Generation

// 只有 generation 变化时才重绘
if self.last_drawn_generation != self.editor.generation() {
    self.scene.reset();
    self.editor.draw(&mut self.scene, transform);
}
```

## 后续步骤

### 1. 平台输入接入

**macOS:**
```rust
// 在 Mac 窗口事件中
WindowEvent::KeyboardInput { event, .. } => {
    let key_event = convert_winit_key(event);
    editor.handle_keyboard(&key_event);
}

WindowEvent::MouseInput { button: MouseButton::Left, state, .. } => {
    if state == Pressed {
        editor.handle_pointer(&pointer_event);
    }
}
```

**Android:**
```rust
// 在 Android 输入回调中
fn on_key_event(&mut self, key_code: KeyCode, down: bool) {
    let event = convert_android_key(key_code, down);
    editor.handle_keyboard(&event);
}
```

**Web:**
```rust
// 在 JavaScript 事件监听中
canvas.addEventListener('keydown', (e) => {
    const event = convert_web_key(e);
    wasm_editor.handle_keyboard(event);
});
```

### 2. 与 dyxel-view 的 Text 组件整合

需要扩展 Text 组件支持编辑状态：

```rust
// dyxel-view/src/lib.rs
pub struct Text { 
    pub id: u32,
    pub editable: bool,  // 新增
}

impl Text {
    pub fn editable(self, editable: bool) -> Self {
        // 设置编辑标志
        self
    }
}
```

### 3. 事件传递到 Guest (WASM)

如果需要让 guest 代码处理编辑事件：

```rust
// 新增 protocol opcodes
[80] EditorInsertText(id: u32, len: u32),
[81] EditorDeleteText(id: u32, start: u32, end: u32),
[82] EditorMoveCursor(id: u32, position: u32),
```

### 4. 多行文本和自动换行

已经支持通过 `set_width(Some(width))` 启用自动换行。

### 5. 字体加载

```rust
// 加载自定义字体
let font_data = std::fs::read("font.ttf")?;
let font = parley::Font::from_data(font_data.into());
editor.font_cx.collection.register_font(font);
```

## 已知限制

1. **字体**: 目前使用系统默认字体，需要实现自定义字体加载
2. **IME**: 尚未实现输入法编辑器支持（中文/日文/韩文）
3. **剪贴板**: 尚未集成系统剪贴板
4. **撤销/重做**: Parley 的 PlainEditor 内置支持，需要暴露 API

## 调试技巧

```rust
// 打印布局信息
let (w, h) = editor.layout_size();
println!("Layout size: {}x{}", w, h);

// 打印光标位置
if let Some(cursor) = editor.cursor_geometry(1.0) {
    println!("Cursor: {:?}", cursor);
}
```
