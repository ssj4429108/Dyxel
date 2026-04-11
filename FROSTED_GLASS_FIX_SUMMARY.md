# 毛玻璃效果修复总结

## 问题描述
毛玻璃效果（Frosted Glass）在 layer_effects_demo 中无法正确渲染，文字不显示且效果不符合预期。

## 修复内容

### 1. 添加 text_color 字段到 ViewNode
**文件**: `crates/dyxel-shared/src/state.rs`
- 在 `ViewNode` 结构体中添加 `text_color: Color` 字段
- 添加 `set_text_color()` 方法用于设置文本颜色
- 默认值为 `Color::BLACK`

### 2. 修复 SetTextColor 命令处理
**文件**: `crates/dyxel-core/src/runtime.rs`
- 修复 `SetTextColor` 命令处理器，使其调用 `state.set_text_color()` 而不是 `set_color_rgba()`

### 3. 修复 Editor 文本颜色渲染
**文件**: `crates/dyxel-editor/src/lib.rs`
- 在 `Editor` 结构体中添加 `text_color: Color` 字段
- 修改 `set_text_color()` 方法正确设置 `text_color` 和 Editor 的 brush
- 修改 `draw()` 方法使用 `self.text_color` 而不是硬编码红色
- 移除 `Editor::new()` 中的默认白色 brush 设置

### 4. 优化示例文字颜色
**文件**: `sample/src/layer_effects_demo.rs`
- 将毛玻璃卡片的文字颜色从灰色 (80,80,80) 改为黑色 (0,0,0) 以提高可见性

## 渲染流程

毛玻璃效果现在使用三阶段渲染:

1. **Pass 1**: 渲染主场景（不包含模糊节点的子元素）
2. **Pass 2**: 为每个模糊节点创建模糊纹理
   - 从场景纹理复制背景区域
   - 应用高斯模糊
3. **Pass 3**: 渲染子元素到独立纹理
   - 文字和子视图渲染到 `children_texture`
   - 使用正确的 `text_color`
4. **合成阶段**: 将所有层合成到最终画面
   - 先绘制主场景
   - 叠加模糊纹理（带透明度和遮罩）
   - 最后叠加子元素纹理（使用 Alpha Blending）

## 调试功能

添加了纹理保存功能用于调试（仅在非 WASM 环境下）:
- 设置环境变量 `DYXEL_DEBUG_FRAMES=1` 启用
- 纹理保存到 `debug_frames/` 目录
- 包含多个阶段的纹理：
  - `pass0_composite`: 最终合成结果
  - `pass1_scene`: 主场景
  - `pass2_blur_view_*`: 模糊纹理
  - `pass3_children`: 子元素纹理

## 验证结果

从调试纹理可以确认:
- ✅ 模糊效果正常工作
- ✅ 子元素（文字）正确渲染到独立纹理
- ✅ 文字颜色正确应用
- ✅ 子元素正确合成到最终画面
- ✅ 透明度和混合效果正确

## 关键代码位置

- `crates/dyxel-editor/src/lib.rs:70-76` - Editor text_color 设置
- `crates/dyxel-editor/src/lib.rs:370-375` - Editor draw 使用 text_color
- `crates/dyxel-shared/src/state.rs:85-90` - ViewNode text_color 字段
- `crates/dyxel-core/src/runtime.rs` - SetTextColor 命令处理
- `crates/dyxel-render-vello/src/lib.rs:1256-1300` - Pass 3 子元素渲染
