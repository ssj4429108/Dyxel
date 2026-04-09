# TextInput 自动化测试指南

## 测试类型

### 1. 单元测试 (`cargo test -p dyxel-core`)
测试状态管理和业务逻辑，不依赖渲染。

```bash
cargo test -p dyxel-core text_input
```

### 2. 集成测试 (`cargo test --test text_input_integration`)
测试完整的用户操作流程。

```bash
# 运行所有场景
cargo test --test text_input_integration

# 运行单个场景
cargo test --test text_input_integration test_basic_render_only
```

### 3. 视觉回归测试
需要人工验证的测试，生成截图进行对比。

## 手动验证清单

在修复渲染问题后，请按以下清单验证：

### BasicRender 场景
- [ ] 背景色正确显示（白色 #FFFFFF）
- [ ] 边框可见（如果设置了 border_width）
- [ ] Placeholder 文字显示为灰色（#666666CC）
- [ ] Placeholder 文字在 focus 后消失

### FocusToggle 场景
- [ ] 点击 Input 获取焦点
- [ ] Focus 边框显示为 iOS 蓝色（#007AFF）
- [ ] 光标开始闪烁
- [ ] 点击外部失去焦点
- [ ] Focus 边框消失

### TextInput 场景
- [ ] 可以输入字符
- [ ] 输入的字符正确显示
- [ ] 光标随输入移动
- [ ] Backspace 删除字符

## 调试技巧

### 查看渲染日志
```bash
RUST_LOG=debug cargo run -p dyxel-mac 2>&1 | grep -E "(render|TextInput|focus)"
```

### 检查 TextInput 状态
```bash
RUST_LOG=trace cargo run -p dyxel-mac 2>&1 | grep -E "(TextInputManager|sync_to_renderer)"
```

### 验证编辑器内容
在 `crates/dyxel-render-vello/src/lib.rs` 添加调试输出：
```rust
println!("Editor {} text: '{}'", id, editor.text());
```

## 常见问题

### Q: 背景色不显示
A: 检查 `render_node_recursive_with_transform` 中 Input 节点的背景渲染逻辑

### Q: 文本不显示
A: 检查：
1. Editor 是否正确创建（ViewType::Input）
2. text_input_states 是否包含该节点的 text
3. Editor 的 text 是否正确设置

### Q: Placeholder 不显示
A: 检查：
1. text_input_states 是否包含 placeholder
2. 条件判断：`editor.text().is_empty() && !text_input.focused`

### Q: Focus 边框不显示
A: 检查：
1. text_input.focused 是否为 true
2. render_focus_border 是否被调用
3. 边框颜色是否正确

### Q: 光标不显示
A: 检查：
1. text_input.cursor_visible 是否为 true（需要 blink 定时器）
2. render_cursor 是否被调用
3. 光标位置计算是否正确
