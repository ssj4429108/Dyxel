# Dyxel IDE 配置指南 - rust-analyzer 原生支持

无需安装 VS Code 扩展，通过 rust-analyzer 配置实现良好的 IDE 支持。

## 快速配置

### 1. 项目级配置（已创建 `.rust-analyzer.toml`）

```toml
[procMacro]
enable = true
attributes.enable = true

[cargo]
buildScripts.enable = true
```

### 2. 用户级配置（可选）

在 `~/.config/rust-analyzer/rust-analyzer.toml`（Linux/Mac）
或 `%APPDATA%/rust-analyzer/rust-analyzer.toml`（Windows）：

```toml
[procMacro]
enable = true
```

## 使用技巧

### 技巧 1: 宏展开查看

**查看 rsx! 宏展开后的代码：**

1. 选中 `rsx! { ... }` 块
2. 按 `Ctrl+Shift+P`（或 `Cmd+Shift+P`）
3. 输入：`Rust Analyzer: Expand macro recursively`
4. 查看展开后的 Rust 代码

### 技巧 2: 跳转到定义

**从 RSX 跳转到组件定义：**

1. 将光标放在 `View`、`Text` 或 `Button` 上
2. 按 `F12` 或 `Ctrl+Click`
3. 跳转到 `dyxel-view/src/lib.rs` 中的定义

如果无法跳转，先执行 **宏展开查看**，然后在展开的代码中跳转。

### 技巧 3: 悬停文档

**查看组件文档：**

1. 将光标悬停在组件名上（如 `View`）
2. 等待 rust-analyzer 显示文档提示

### 技巧 4: 属性补全

**使用原生 API 获得完整补全：**

```rust
// 使用原生 API（IDE 友好）
let view = View::new()
    .width("100%")      // <- 输入 .w 后按 Tab 补全
    .height("100%")     // <- 有完整类型提示
    .color((50, 50, 50));

// 然后在 rsx! 中使用
rsx! {
    view  // 预配置的视图
}
```

## 推荐的代码组织

### 模式 1: 原生 API + rsx! 布局（推荐）

```rust
use dyxel_app::prelude::*;

#[app]
pub fn MyApp() -> impl BaseView {
    // IDE 友好的原生 API 配置
    let header = Text::new()
        .value("标题".to_string())
        .font_size(24.0)
        .text_color((255, 255, 255, 255));
    
    let content = View::new()
        .width("100%")
        .height(200.0)
        .color((60, 60, 60));
    
    // 简洁的 rsx! 布局
    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (30, 30, 30),
            
            header
            content
        }
    }
}
```

### 模式 2: 完全原生 API

```rust
#[app]
pub fn MyApp() -> impl BaseView {
    let text = Text::new()
        .value("Hello".to_string())
        .font_size(24.0);
    
    View::new()
        .width("100%")
        .height("100%")
        .color((50, 50, 50))
        .child(text.node_id())
}
```

## rust-analyzer 命令速查

| 命令 | 快捷键 | 作用 |
|-----|-------|------|
| Expand macro | - | 展开宏查看生成的代码 |
| Go to Definition | `F12` | 跳转到定义 |
| Find All References | `Shift+F12` | 查找所有引用 |
| Hover | 悬停 | 查看类型和文档 |
| Rename | `F2` | 重命名符号 |
| Format Document | `Shift+Alt+F` | 格式化代码 |

## 故障排除

### 问题 1: 宏无法展开

**解决：**
```bash
cargo clean
rust-analyzer restart
```

### 问题 2: 跳转到定义不工作

**解决：**
1. 确保项目已构建：`./build_android.sh`
2. 检查 `.rust-analyzer.toml` 存在
3. 重启 rust-analyzer

### 问题 3: 代码补全不工作

**解决：**
1. 等待 rust-analyzer 索引完成（看状态栏）
2. 保存文件触发检查
3. 确保 `Cargo.toml` 依赖正确

## 高级配置

### 自定义宏展开深度

在 `.rust-analyzer.toml`：

```toml
[hover]
actions.enable = true

[completion]
enable = true
autoimport.enable = true
```

### 性能优化

```toml
[cargo]
# 减少检查目标
features = ["wasm"]
```

## 总结

rust-analyzer 原生支持通过以下方式使用 Dyxel：

1. ✅ **宏展开查看** - 理解 rsx! 生成什么代码
2. ✅ **原生 API 跳转** - 完全支持 Go to Definition
3. ✅ **原生 API 补全** - 完整的代码补全
4. ⚠️ **宏内跳转** - 有限支持，建议用原生 API 配置

推荐：**原生 API 配置组件 + rsx! 组合布局**
