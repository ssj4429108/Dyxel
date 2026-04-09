# Product Backlog

## 高优先级 (High Priority)

### Story 1: 完善基础 UI 组件系统 ✅ 已完成
- **状态**: ✅ Done
- **描述**: 作为开发者，我希望能使用基础 UI 组件（Button、TextInput、Flex 布局）来构建界面
- **AC**:
  - ✅ TextInput: 基础渲染、混合布局、RSX 语法、样式设置、手势支持
  - ✅ TextInput: 光标闪烁效果 (530ms 间隔)
  - ✅ TextInput: 文本选择高亮
  - ✅ TextInput: Placeholder 支持
  - ✅ Button: 点击事件处理、5种变体、样式定制、状态反馈
  - ✅ Flex: Column/Row 布局、对齐方式、间距控制
- **预估**: 13 点
- **完成日期**: 2026-04-08

## 中优先级 (Medium Priority)

### Story 2: 手势系统优化
- **状态**: 📋 Todo
- **描述**: 优化手势识别系统的性能和准确性
- **AC**:
  - 支持 Pan、Scale、Rotation 等基础手势
  - 手势冲突解决机制
  - 性能优化（渲染循环内无堆分配）
- **预估**: 8 点

## 低优先级/想法 (Low Priority / Ideas)

### Idea 1: Web 端支持完善
- **状态**: 💡 Idea
- **描述**: 完善 Web (WASM) 端的渲染和交互支持
- **备注**: 等待 WGPU Web 支持稳定

### Idea 2: 开发者工具
- **状态**: 💡 Idea
- **描述**: 提供布局调试、性能监控等开发者工具
