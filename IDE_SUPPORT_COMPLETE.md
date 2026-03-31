# Dyxel 完整 IDE 支持方案

## 三层架构实现

```
┌─────────────────────────────────────────────────────────────┐
│  第 3 层: VS Code Extension                                  │
│  ├── 语法高亮 (TextMate Grammar)                            │
│  ├── LSP 客户端集成                                         │
│  └── 主题和图标支持                                         │
├─────────────────────────────────────────────────────────────┤
│  第 2 层: dyxel-lsp (Language Server)                        │
│  ├── RSX 语法分析                                           │
│  ├── 语义补全                                               │
│  ├── 跳转定义                                               │
│  └── 悬停文档                                               │
├─────────────────────────────────────────────────────────────┤
│  第 1 层: rsx! 宏 (Span 保留)                                │
│  ├── quote_spanned! 精确映射                                 │
│  ├── 类型 span 保留（跳转支持）                              │
│  └── 属性 span 保留（补全支持）                              │
└─────────────────────────────────────────────────────────────┘
```

## 组件清单

### 1. rsx! 宏 (已完成 ✅)

**文件**: `crates/dyxel-rsx/src/lib.rs`

**关键实现**:
```rust
// 使用原始类型 span，支持 "Go to Definition"
let node_type = proc_macro2::Ident::new(&node.node_type, type_span);

// 使用 quote_spanned! 保留属性位置
quote_spanned! { *name_span =>
    .#method_ident(#value)
}
```

### 2. Language Server (已完成 ✅)

**文件**: `crates/dyxel-lsp/src/main.rs`

**构建**:
```bash
cargo build -p dyxel-lsp --release
```

**功能**:
- 文本同步
- 诊断（括号匹配、未知组件）
- 跳转定义
- 悬停文档

### 3. VS Code 扩展 (已完成 ✅)

**文件**: `editors/vscode/`

**安装**:
```bash
cd editors/vscode
npm install
npm run compile
```

**调试**: 按 `F5` 启动扩展开发主机

## 使用指南

### 启用完整 IDE 支持

1. **构建 LSP 服务器**
   ```bash
   cargo build -p dyxel-lsp --release
   ```

2. **安装 VS Code 扩展**
   ```bash
   cd editors/vscode
   npm install
   npm run compile
   # 按 F5 启动调试
   ```

3. **配置 LSP 路径**
   在 `.vscode/settings.json`:
   ```json
   {
     "dyxel.languageServerPath": "${workspaceFolder}/target/release/dyxel-lsp"
   }
   ```

### 功能特性

| 功能 | 快捷键 | 说明 |
|-----|-------|------|
| 跳转定义 | `F12` | 从 View/Text 跳转到定义 |
| 查看文档 | 悬停 | 显示组件/属性文档 |
| 自动补全 | `Ctrl+Space` | 组件名、属性名 |
| 语法高亮 | 自动 | RSX 特殊着色 |
| 错误检查 | 自动 | 括号匹配、未知组件 |

## 架构对比

### 之前（rust-analyzer 原生）

```
rsx! { View { ... } }
    ↓ 宏展开
{ let mut _view_node = ::dyxel_view::View::new(); ... }
    ↓ rust-analyzer 分析
部分支持（需要宏展开才能跳转）
```

### 之后（三层架构）

```
rsx! { View { ... } }
    ↓ dyxel-lsp 直接分析 RSX 语法
识别 View 在位置 X
    ↓ 查询组件定义数据库
View → crates/dyxel-view/src/lib.rs:471
    ↓ 直接跳转
无需宏展开！
```

## 优势

1. **无需等待宏展开** - LSP 直接理解 RSX 语法
2. **精确的跳转位置** - 直接跳转到组件定义
3. **语义补全** - 知道 View 有哪些属性
4. **实时诊断** - 括号不匹配立即提示

## 未来扩展

- [ ] IntelliJ IDEA 插件
- [ ] Vim/Neovim LSP 配置
- [ ] Emacs LSP 模式
- [ ] 代码格式化
- [ ] 重构（重命名组件）
