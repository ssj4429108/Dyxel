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

## 2. 潜在风险与技术难点

### 2.1 布局计算的复杂性 (P0)

**风险:** 在 TextInput 中，内边距不仅影响容器外观，还直接影响文本绘制的起始坐标以及点击测试（Hit Testing）的偏移量。

**缓解措施:**
- 在 Host 端维护清晰的坐标系：
  - 容器坐标系：原点在容器左上角
  - 内容坐标系：原点在 padding 后的文本区域左上角
- `text_origin` 计算公式必须在 `TextInputManager` 中明确定义
- 任何 padding 变化必须触发重新计算

### 2.2 性能开销 (P1)

**风险:** 增加光标阴影（blur_radius）和圆角会增加 Vello 的渲染层级。如果一个界面有大量带阴影的输入框，可能会引起掉帧。

**缓解措施:**
- **条件渲染:** 阴影只在 `focused && cursor_visible` 时才进行昂贵的模糊计算
- **缓存策略:** 使用 `generation` 机制，无变化时跳过阴影重新计算
- **降级策略:** 在低性能设备上自动禁用阴影

### 2.3 文本选择的一致性 (P2)

**风险:** SelectableText 和 TextInput 共享渲染代码很好，但在交互逻辑上（例如双击选词、三击全选）可能存在细微差别。

**缓解措施:**
- 在 `dyxel-core` 层抽象出 `SelectionManager` 类（见 3.8 节）
- 统一处理逻辑索引（usize）到屏幕坐标（f32）的转换
- 渲染层只接收最终的选择区域矩形，不关心交互细节

### 2.4 光标边界剪裁 (P1)

**风险:** 当文本达到容器最右侧且 `padding-right` 为 0 时，光标可能被容器边界剪裁（Clipped），导致用户看不到光标位置。

**缓解措施:**
- 在 `SelectionManager` 的坐标计算中增加 `cursor_width / 2.0` 的安全余量（见 3.9 节）
- 实现自动水平滚动机制，当光标接近边界时滚动文本
- 渲染时确保光标矩形不完全依赖内容区域，允许略微超出

---

## 3. 设计细节

### 3.1 内边距支持

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

**重要区分:**
- **Content Padding:** 文本绘制区域的内边距，影响 `text_origin` 计算
- **Hit Area:** 点击测试区域，应覆盖整个容器（包括 padding 区域）

**布局计算公式 (Host 端):**
```rust
// 容器总尺寸 (由 Taffy 布局计算)
let container_size = layout.size;

// 内容区域 = 容器 - padding
let content_rect = Rect {
    x: padding_left,
    y: padding_top,
    width: container_size.width - padding_left - padding_right,
    height: container_size.height - padding_top - padding_bottom,
};

// 文本绘制起始坐标 (考虑基线对齐)
let text_origin = Point {
    x: content_rect.x,
    y: content_rect.y + baseline_offset, // 垂直居中
};

// 点击测试区域 (整个容器)
let hit_area = container_size.to_rect();
```

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

**布局同步机制:**
当 font_size 或 placeholder_font_size 改变时，需要触发 Guest 端重新布局：
1. Host 检测到样式变化 → 标记 layout_dirty
2. 下帧同步时发送 `LayoutChanged` 事件到 Guest
3. Guest 收到后重新查询布局信息

---

### 3.2 光标自定义

**结构:**
```rust
pub struct CursorStyle {
    pub width: f32,           // 默认: 2.0
    pub color: [u8; 4],       // 默认: 继承文字颜色
    pub radius: f32,          // 默认: 0.0 (矩形), 典型值: 1.0-2.0
    pub blink_interval_ms: u64, // 默认: 530 (iOS 风格)
    pub shadow: Option<ShadowStyle>, // 默认: None
}

pub struct ShadowStyle {
    pub color: [u8; 4],       // 默认: 光标颜色 30% 透明度
    pub blur_radius: f32,     // 默认: 4.0
    pub offset_x: f32,        // 默认: 0.0
    pub offset_y: f32,        // 默认: 0.0
}
```

**新增协议指令 (原子化设计):**

| OpCode | 名称 | 参数 | 说明 |
|--------|------|------------|------|
| 134 | SetTextInputCursorStyle | id, width, radius | 光标几何样式 |
| 134A | SetTextInputCursorColor | id, r, g, b, a | 光标颜色 (主题切换专用) |
| 135 | SetTextInputCursorShadow | id, blur, offset_x, offset_y, r, g, b, a | 光标阴影 |
| 136 | SetTextInputCursorBlinkInterval | id, interval_ms | 闪烁间隔 |

**设计理由:**
- **原子化:** 将颜色独立出来，方便 Dark/Light 主题切换时只更新颜色而不重新发送完整样式包
- **性能优化:** 阴影只在 `focused && cursor_visible` 时才进行昂贵的模糊计算
- **渲染实现:** 阴影作为独立绘图操作而非 Layer 效果，减少离屏渲染开销

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

### 3.8 选择管理器 (SelectionManager)

为统一 SelectableText 和 TextInput 的选择逻辑，在 `dyxel-core` 层抽象出 SelectionManager：

```rust
/// 处理文本选择的通用逻辑
pub struct SelectionManager {
    /// 逻辑索引 -> 屏幕坐标的转换
    pub fn index_to_position(
        &self,
        text: &str,
        index: usize,
        layout: &TextLayout,
    ) -> Point;

    /// 屏幕坐标 -> 逻辑索引的转换
    pub fn position_to_index(
        &self,
        text: &str,
        position: Point,
        layout: &TextLayout,
    ) -> usize;

    /// 双击选词
    pub fn select_word(&self, text: &str, index: usize) -> (usize, usize);

    /// 三击全选
    pub fn select_all(&self, text: &str) -> (usize, usize);

    /// 获取选区的屏幕矩形（用于高亮绘制）
    pub fn selection_rects(
        &self,
        text: &str,
        start: usize,
        end: usize,
        layout: &TextLayout,
    ) -> Vec<Rect>;
}
```

**光标占位安全余量 (Cursor Inset):**
```rust
/// 计算文本绘制的安全区域，确保光标不被容器边界剪裁
pub fn content_inset(&self, cursor_width: f32, padding: [f32; 4]
) -> [f32; 4] {
    let cursor_inset = cursor_width / 2.0; // 光标宽度的一半作为安全余量
    [
        padding[0],                      // top
        padding[1] + cursor_inset,       // right (关键：右侧增加余量)
        padding[2],                      // bottom
        padding[3],                      // left
    ]
}
```

**组合键支持接口:**
```rust
/// 键盘修饰符状态
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,   // Windows/Linux
    pub alt: bool,
    pub meta: bool,   // macOS Cmd
}

impl SelectionManager {
    /// 处理带修饰符的方向键（Shift + 方向键选择）
    pub fn handle_arrow_key(
        &mut self,
        direction: ArrowDirection,
        modifiers: KeyModifiers,
        current_selection: (usize, usize),
    ) -> (usize, usize) {
        if modifiers.shift {
            // 扩展选区
            self.extend_selection(direction, current_selection)
        } else {
            // 移动光标
            self.move_cursor(direction, current_selection)
        }
    }

    /// Ctrl/Cmd + A 全选
    pub fn handle_select_all(&self, text: &str, modifiers: KeyModifiers
    ) -> Option<(usize, usize)> {
        if modifiers.ctrl || modifiers.meta {
            Some(self.select_all(text))
        } else {
            None
        }
    }
}
```

**共享逻辑:**
- TextInput 和 SelectableText 都使用 SelectionManager 处理交互
- 渲染层只接收最终的 `selection_rects` 进行绘制

---

### 3.9 光标占位与边界处理

**问题:** 当文本到达容器最右侧且 `padding-right` 为 0 时，光标可能被容器边界剪裁。

**解决方案:**

```rust
/// TextInput 渲染时的光标边界保护
pub struct CursorBoundaryProtection {
    /// 光标安全余量（默认 cursor_width / 2.0）
    pub inset: f32,
    /// 是否启用自动滚动（当光标接近边界时）
    pub auto_scroll: bool,
    /// 边界触发阈值（像素）
    pub threshold: f32,
}

impl TextInputRenderState {
    /// 计算实际内容区域（考虑光标占位）
    pub fn effective_content_rect(&self,
        container_rect: Rect,
    ) -> Rect {
        let inset = self.cursor_style.width / 2.0;
        Rect {
            x: container_rect.x + self.padding[3], // left
            y: container_rect.y + self.padding[0], // top
            width: container_rect.width
                - self.padding[1] - self.padding[3] // right - left
                - inset, // 光标安全余量
            height: container_rect.height
                - self.padding[0] - self.padding[2], // top - bottom
        }
    }

    /// 检查光标是否接近右边界，需要自动滚动
    pub fn should_auto_scroll(
        &self,
        cursor_x: f32,
        visible_width: f32,
    ) -> bool {
        cursor_x > visible_width - self.cursor_boundary.threshold
    }
}
```

**渲染策略:**
1. 计算 `effective_content_rect` 时右侧减去 `cursor_width / 2.0`
2. 当光标位置接近边界时触发水平滚动
3. 确保光标始终可见，不被容器边界剪裁

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
    pub hit_area: Rect,    // 点击测试区域（包含 padding 的整个容器）
}

/// 点击测试结果
pub struct HitTestResult {
    pub node_id: u32,
    pub local_position: Point, // 相对于内容区域的坐标
    pub in_content_area: bool, // 是否在内容区域内（用于光标定位）
}
```

---

## 5. 给 Claude Code 的实施提示

### 5.1 Vello 渲染注意事项

**光标阴影实现:**
在 `render_cursor` 函数中绘制阴影时，使用 `vello::kurbo::Rect` 的 `inset` 方法来配合定义的 `cursor_radius`：

```rust
fn render_cursor_with_shadow(
    builder: &mut Scene,
    transform: Affine,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
    color: Color,
    shadow: &ShadowStyle,
) {
    // 1. 先绘制阴影（在光标下方）
    let shadow_rect = RoundedRect::new(
        x + shadow.offset_x as f64,
        y + shadow.offset_y as f64,
        x + width + shadow.offset_x as f64,
        y + height + shadow.offset_y as f64,
        radius,
    );
    // 使用 blur 效果渲染阴影
    
    // 2. 绘制光标主体
    let cursor_rect = RoundedRect::new(x, y, x + width, y + height, radius);
    builder.fill(Fill::NonZero, transform, color, None, &cursor_rect);
}
```

**关键要点:**
- 阴影必须在光标主体之前绘制（ painters algorithm ）
- 使用 `inset` 确保圆角矩形正确缩进
- 阴影只在 `focused && cursor_visible` 时计算，避免不必要的性能开销

### 5.2 SelectionManager 实施顺序

**建议采用 TDD 方式：**

1. **先在 `dyxel-core` 中独立实现 `SelectionManager`**
   - 不依赖任何 UI 库或渲染代码
   - 纯数学逻辑：逻辑索引 (usize) ↔ 屏幕坐标 (f32)

2. **编写全量单元测试**
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;
       
       #[test]
       fn test_index_to_position_ascii() {
           // 测试 ASCII 文本索引转换
       }
       
       #[test]
       fn test_index_to_position_cjk() {
           // 测试中日韩文本（多字节字符）
       }
       
       #[test]
       fn test_position_to_index_boundary() {
           // 测试边界条件：行首、行尾、空文本
       }
       
       #[test]
       fn test_select_word() {
           // 测试双击选词：标点、空格、换行符处理
       }
       
       #[test]
       fn test_cursor_inset_calculation() {
           // 测试光标安全余量计算
           let sm = SelectionManager::new();
           let inset = sm.content_inset(2.0, [12.0, 16.0, 12.0, 16.0]);
           assert_eq!(inset[1], 16.0 + 1.0); // right padding + cursor_width/2
       }
   }
   ```

3. **数学逻辑 100% 验证后再集成到渲染管线**
   - 使用属性测试（property-based testing）验证反函数性质：
     `position_to_index(index_to_position(i)) == i`

### 5.3 开发顺序建议

```
Phase 0: SelectionManager (纯数学逻辑 + 全量测试)
    ↓
Phase 1: RSX 修复 + Padding 支持
    ↓
Phase 2: 基础光标样式（宽、色、圆角）
    ↓
Phase 3: 阴影 + Placeholder 样式
    ↓
Phase 4: 容器样式 + 选区颜色
    ↓
Phase 5: Text 选择模式 + 组合键支持
```

---

## 6. 实施计划摘要

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

## 7. 文件变更清单

| 文件 | 变更 |
|------|--------|
| `dyxel-shared/src/protocol.rs` | 添加新 OpCodes 130-141 |
| `dyxel-view/src/components/text_input.rs` | 添加新 Builder 方法，修复 RSX |
| `dyxel-view/src/lib.rs` (Text) | 添加 selectable API |
| `dyxel-render-api/src/lib.rs` | 扩展 TextInputRenderState |
| `dyxel-render-vello/src/lib.rs` | 更新 render_cursor，添加阴影支持 |
| `dyxel-core/src/text_input/manager.rs` | 处理新协议指令 |

---

## 8. 向后兼容性

所有新功能都是增量的:
- 新 OpCodes 不影响现有指令处理
- 默认样式与当前行为一致 (黑色文字、16px、2px 光标)
- Text 选择通过 `selectable(true)` 显式启用

---

## 9. 测试策略

1. **单元测试:** 协议编码/解码
2. **集成测试:** WASM API 生成正确的指令序列
3. **视觉测试:** 渲染输出与预期的光标/选择外观一致
4. **交互测试:** Text 长按触发选择模式

---

## 10. 待确认问题 (已解决)

| 问题 | 决策 | 理由 |
|------|------|------|
| 光标阴影归属 | 作为 `CursorStyle` 的一部分 | Host 端将其实现为**独立绘图操作**而非 Layer 效果，减少离屏渲染开销 |
| 内容内边距 | **暂不需要独立** | 当前 padding 方案足以覆盖 90% 的场景，保持简单性 |
| 选择手柄 | **自定义渲染** | Dyxel 追求跨平台一致性，原生手柄在不同系统版本外观不一，且与 Vello 渲染管线集成困难 |

**补充确认:**
- 协议原子性：已将光标颜色独立为 `SetTextInputCursorColor` (134A)，方便主题切换
- 点击区域：明确 `ContainerStyle.hit_area` 覆盖整个容器（含 padding）
- 布局同步：样式改变后通过 `LayoutChanged` 事件触发 Guest 端重新布局

---

## 11. 参考资料

- Flutter TextField: https://api.flutter.dev/flutter/material/TextField-class.html
- Flutter InputDecoration: https://api.flutter.dev/flutter/material/InputDecoration-class.html
- Flutter SelectableText: https://api.flutter.dev/flutter/material/SelectableText-class.html
- iOS UITextField 样式指南
