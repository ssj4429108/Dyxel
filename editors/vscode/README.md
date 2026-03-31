# Dyxel VS Code Extension

Dyxel RSX 语言支持扩展

## 功能

- 🎯 **RSX 语法高亮** - 组件、属性特殊着色
- 🔍 **跳转定义** - Ctrl+Click 跳转到组件定义
- 💡 **自动补全** - 组件名和属性提示
- 📖 **悬停文档** - 查看组件和属性文档

## 安装

### 1. 构建语言服务器

```bash
cd /path/to/dyxel
cargo build -p dyxel-lsp --release
```

### 2. 安装扩展

```bash
cd editors/vscode
npm install
npm run compile
```

然后按 `F5` 启动扩展开发主机，或打包安装：

```bash
vsce package
```

在 VS Code 中安装生成的 `.vsix` 文件。

## 配置

在 `.vscode/settings.json`：

```json
{
  "dyxel.languageServerPath": "/path/to/dyxel-lsp",
  "dyxel.enableLanguageServer": true
}
```

## 使用

在 `rsx!` 宏中：

- **组件跳转**: 将光标放在 `View`、`Text` 等组件上，按 `F12` 跳转
- **属性补全**: 输入 `wid` 后按 `Tab` 补全 `width`
- **悬停文档**: 悬停在属性名上查看说明

## 支持的组件

- `View` - 容器视图
- `Text` - 文本
- `Button` - 按钮
- `Column` - 垂直布局
- `Row` - 水平布局
