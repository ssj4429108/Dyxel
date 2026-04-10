# TextInput 组件增强设计文档

**日期:** 2026-04-10  
**状态:** 待实现  
**作者:** Claude Code

---

## 1. 概述

本文档详细说明了 Dyxel TextInput 组件和 Text 组件选择能力的增强设计。目标是在保持 Dyxel "Thin Guest, Thick Host" 架构的前提下，使 TextInput 更接近 Flutter TextField 的功能。

### 1.1 范围

- **TextInput 样式:** 内边距、光标自定义、颜色、边框
- **Text 选择:** 为 Text 组件启用长按选择功能
- **RSX 宏修复:** 确保显式设置的样式覆盖默认值

### 1.2 非目标

- 富文本编辑（单个输入框内的多种样式）
- 复杂的输入验证
- 自动完成/建议 UI
- 多行文本输入扩展（TextArea 是未来的工作）

---

## 2. 架构原则

1. **复用现有基础设施:** 使用 `SetPadding` (OpCode 13) 实现容器内边距
2. **扩展而非替换:** 添加新协议指令而非破坏现有协议
3. **Host 端渲染:** 所有视觉效果（阴影、圆角）在 Host (Vello) 中计算
4. **Guest 端 API:** WASM 提供 Builder 模式 API，转换为协议指令

---

## 3. 设计细节

### 3.1 内边距支持

**决策:** 复用现有的 `SetPadding` 指令 (OpCode 13) 实现容器内边距。

**WASM API:**
```rust
impl TextInput {
    pub fn padding(self, padding: impl Into<Prop<(f32, f32, f32, f32)>>) -> Self;
    pub fn padding_horizontal(self, value: f32) -> Self;
    pub fn padding_vertical(self, value: f32) -> Self;
    pub fn padding_all(self, value: f32) -> Self;
}
```

**默认值:** `(12.0, 16.0, 12.0, 16.0)` (上、右、下、左) - iOS 风格的舒适间距

---

### 3.2 光标自定义

**结构:**
```rust
pub struct CursorStyle {
    pub width: f32,           // 默认: 2.0
    pub color: [u8; 4],       // 默认: 继承文字颜色
    pub radius: f32,          // 默认: 0.0 (矩形), 典型值: 1.0-2.0
    pub blink_interval_ms: u64, // 默认: 530 (iOS 风格)
}

pub struct ShadowStyle {
    pub color: [u8; 4],       // 默认: 光标颜色 30% 透明度
    pub blur_radius: f32,     // 默认: 4.0
    pub offset_x: f32,        // 默认: 0.0
    pub offset_y: f32,        // 默认: 0.0
}
```

**新增协议指令:**

| OpCode | 名称 | 参数 |
|--------|------|------------|
| 134 | SetTextInputCursorStyle | id, width, radius, r, g, b, a |
| 135 | SetTextInputCursorShadow | id, blur, offset_x, offset_y, r, g, b, a |
| 136 | SetTextInputCursorBlinkInterval | id, interval_ms |

**WASM API:**
```rust
impl TextInput {
    pub fn cursor_width(self, width: f32) -> Self;
    pub fn cursor_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
    pub fn cursor_radius(self, radius: f32) -> Self;
    pub fn cursor_blink_interval(self, ms: u32) -> Self;
    pub fn cursor_shadow(self, shadow: ShadowStyle) -> Self;
    
    // 便捷方法: 一次性设置所有属性
    pub fn cursor_style(self, style: CursorStyle) -> Self;
}
```

**渲染:**
- 光标绘制为填充矩形，支持圆角
- 阴影使用 Vello 的阴影/模糊效果
- 闪烁状态由 Host 管理 (TextInputManager::update_cursor_blink)

---

### 3.3 文字样式 (RSX 修复)

**问题:** TextInput::new() 中硬编码了默认值:
```rust
push_command!(SHARED_BUFFER, SetTextColor, id, 0u8, 0u8, 0u8, 255u8);
push_command!(SHARED_BUFFER, SetFontSize, id, 16.0_f32);
```

**解决方案:** 延迟应用默认值

```rust
pub struct TextInput {
    id: u32,
    placeholder_text: Option<String>,
    // 跟踪哪些样式已被显式设置
    explicit_styles: RefCell<ExplicitStyles>,
}

struct ExplicitStyles {
    font_size: bool,
    text_color: bool,
    // ... 等等
}

impl TextInput {
    pub fn new() -> Self {
        // 不要在这里应用默认值
        // 只创建节点
    }
    
    pub fn font_size(self, size: impl Into<Prop<f32>>) -> Self {
        self.explicit_styles.borrow_mut().font_size = true;
        // ... 应用属性
        self
    }
    
    // 在首次渲染前调用
    pub(crate) fn apply_defaults(&self) {
        let explicit = self.explicit_styles.borrow();
        if !explicit.font_size {
            self.apply_font_size(16.0);
        }
        if !explicit.text_color {
            self.apply_text_color((0, 0, 0, 255));
        }
        // ... 等等
    }
}
```

---

### 3.4 Placeholder 样式

目前 placeholder 继承文字样式，需要独立的样式控制。

**新增协议:**

| OpCode | 名称 | 参数 |
|--------|------|------------|
| 131 | SetTextInputPlaceholderStyle | id, r, g, b, a, font_size |

**WASM API:**
```rust
impl TextInput {
    pub fn placeholder_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
    pub fn placeholder_font_size(self, size: impl Into<Prop<f32>>) -> Self;
    
    // 已有方法
    pub fn placeholder(self, text: impl Into<String>) -> Self;
}
```

**默认值:** 颜色 `#999999` (柔和灰色), 字号与文字相同

---

### 3.5 容器样式

输入框容器的背景色、边框和圆角。

**新增协议:**

| OpCode | 名称 | 参数 |
|--------|------|------------|
| 132 | SetTextInputBackgroundColor | id, r, g, b, a |
| 133 | SetTextInputBorderStyle | id, style, width, r, g, b, a |

**WASM API:**
```rust
impl TextInput {
    pub fn background_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
    pub fn border_width(self, width: f32) -> Self;
    pub fn border_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
    pub fn border_radius(self, radius: f32) -> Self;
    
    // 便捷方法
    pub fn border(self, width: f32, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
}
```

**默认值:**
- 背景: 透明 (使用父级背景)
- 边框: 1px 实线 `#E0E0E0`
- 圆角: 8.0 (iOS 风格)

---

### 3.6 选区样式

已选中文本的视觉反馈。

**新增协议:**

| OpCode | 名称 | 参数 |
|--------|------|------------|
| 137 | SetTextInputSelectionColor | id, r, g, b, a |

**WASM API:**
```rust
impl TextInput {
    pub fn selection_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
}
```

**默认值:** 主色调 30% 透明度 (例如 iOS 上的 `#007AFF4D`)

**渲染:**
- 用半透明颜色填充选中文本区域
- 在选区边界绘制光标

---

### 3.7 Text 组件选择 (SelectableText)

使 Text 组件可被选中文本，类似 Flutter 的 SelectableText。

**新增协议:**

| OpCode | 名称 | 参数 |
|--------|------|------------|
| 140 | SetTextSelectable | id, enabled |
| 141 | SetTextSelection | id, start, end |

**WASM API:**
```rust
impl Text {
    pub fn selectable(self, enabled: bool) -> Self;
    pub fn selection(self, range: impl Into<Prop<(usize, usize)>>) -> Self;
    pub fn selection_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
}
```

**行为:**
- 当 `selectable=true` 时，长按进入选择模式
- 显示选择手柄 (iOS 上的放大镜效果)
- 上下文菜单: 复制、全选
- 无光标闪烁 (选区起点显示静态光标)

**与 TextInput 共享:**
两者在 Vello 后端使用相同的选择渲染代码。

---

## 4. 渲染状态更新

### 4.1 TextInputRenderState (dyxel-render-api)

```rust
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

pub struct PlaceholderStyle {
    pub color: [u8; 4],
    pub font_size: f32,
}

pub struct ContainerStyle {
    pub background_color: [u8; 4],
    pub border_width: f32,
    pub border_color: [u8; 4],
    pub border_radius: f32,
    pub padding: [f32; 4], // 上、右、下、左
}
```

---

## 5. 实施计划摘要

### 阶段 1: 核心修复 (P0)
1. 修复 RSX 宏 - 延迟应用默认值
2. 使用现有 SetPadding 添加内边距支持
3. 基础光标自定义 (宽度、颜色、圆角)

### 阶段 2: 增强样式 (P1)
4. 光标阴影效果
5. Placeholder 独立样式
6. 容器背景/边框/圆角
7. 选区颜色

### 阶段 3: 高级功能 (P2)
8. Text 组件选择模式
9. 选择手柄渲染
10. 动画支持 (聚焦边框过渡)

---

## 6. 文件变更清单

| 文件 | 变更 |
|------|--------|
| `dyxel-shared/src/protocol.rs` | 添加新 OpCodes 130-141 |
| `dyxel-view/src/components/text_input.rs` | 添加新 Builder 方法，修复 RSX |
| `dyxel-view/src/lib.rs` (Text) | 添加 selectable API |
| `dyxel-render-api/src/lib.rs` | 扩展 TextInputRenderState |
| `dyxel-render-vello/src/lib.rs` | 更新 render_cursor，添加阴影支持 |
| `dyxel-core/src/text_input/manager.rs` | 处理新协议指令 |

---

## 7. 向后兼容性

所有新功能都是增量的:
- 新 OpCodes 不影响现有指令处理
- 默认样式与当前行为一致 (黑色文字、16px、2px 光标)
- Text 选择通过 `selectable(true)` 显式启用

---

## 8. 测试策略

1. **单元测试:** 协议编码/解码
2. **集成测试:** WASM API 生成正确的指令序列
3. **视觉测试:** 渲染输出与预期的光标/选择外观一致
4. **交互测试:** Text 长按触发选择模式

---

## 9. 待确认问题

1. 光标阴影应该是 CursorStyle 的一部分还是独立的 Layer 效果?
2. 是否需要独立于容器内边距的内容内边距?
3. 选择手柄应该是平台原生还是自定义渲染?

---

## 10. 参考资料

- Flutter TextField: https://api.flutter.dev/flutter/material/TextField-class.html
- Flutter InputDecoration: https://api.flutter.dev/flutter/material/InputDecoration-class.html
- Flutter SelectableText: https://api.flutter.dev/flutter/material/SelectableText-class.html
- iOS UITextField 样式指南
