# Dyxel Sample - 示例集合

本目录包含多个演示示例，展示 Dyxel 框架的不同功能。

## 示例列表

### 1. transaction_demo.rs - Transaction API 演示
**功能亮点：**
- 使用 Transaction 批量创建节点（减少 Host 往返）
- LayoutRegistry API 读取布局信息
- 文本溢出检测
- 瀑布流布局计算

**适用场景：** 需要批量操作节点、延迟敏感的布局读取

### 2. shadow_layout_demo.rs - Shadow Layout 演示（⭐ 新功能）
**功能亮点：**
- 零延迟布局查询 (`get_layout_estimated`)
- 文本溢出预检测 (`would_text_overflow`)
- 动态字体大小调整
- 瀑布流布局预计算
- 视口大小管理

**适用场景：** 需要立即响应的交互、复杂布局预计算

### 3. combined_demo.rs - 综合演示（⭐ 推荐）
**功能亮点：**
- 三层布局系统协同工作：
  - **Shadow Layer (0ms)**: `get_layout_estimated()`
  - **Registry Layer (16ms)**: `get_layout()` / `take_layout()`
  - **Host Layer**: 最终渲染
- Transaction + Shadow Layout + LayoutRegistry 组合使用
- 对比 Shadow 与 Registry 布局差异

**适用场景：** 了解完整架构、生产环境参考

## 快速开始

### 选择示例
编辑 `src/lib.rs`，取消你想运行的示例注释：

```rust
// 选择示例1: Transaction API
mod transaction_demo;
use transaction_demo as demo;

// 或选择示例2: Shadow Layout
// mod shadow_layout_demo;
// use shadow_layout_demo as demo;

// 或选择示例3: 综合演示
// mod combined_demo;
// use combined_demo as demo;
```

### 构建运行
```bash
# macOS
./build_mac.sh

# Web (WASM)
./build_web.sh

# Android
./build_android.sh

# iOS
./build_ios.sh
```

## API 对比

| 功能 | Transaction Demo | Shadow Layout Demo | Combined Demo |
|------|-----------------|-------------------|---------------|
| 批量创建 | ✅ | ✅ | ✅ |
| 零延迟布局 | ❌ | ✅ | ✅ |
| 文本溢出检测 | ✅ (延迟) | ✅ (即时) | ✅ (两者对比) |
| 瀑布流计算 | ✅ | ✅ | ✅ |
| 视口管理 | ❌ | ✅ | ✅ |

## 关键 API

### Shadow Layout（零延迟）
```rust
// 初始化（必须在创建视图前调用）
init_shadow_tree();

// 零延迟布局查询
let layout = get_layout_estimated(node_id);

// 文本溢出预检测
if would_text_overflow(node_id, text_width) {
    // 调整字体...
}

// 获取预估底部位置（瀑布流）
if let Some(bottom) = get_estimated_bottom_y(node_id) {
    // 使用 bottom 值...
}
```

### LayoutRegistry（一帧延迟）
```rust
// 获取已提交的布局
let layout = get_layout(node_id);

// 获取并清除脏标记
let layout = take_layout(node_id);

// 检查是否有新布局
if is_layout_dirty(node_id) {
    let layout = take_layout(node_id);
}
```

### Transaction（批量操作）
```rust
// 方式1：手动管理
let tx = begin_transaction();
// ... 创建节点 ...
tx.commit();

// 方式2：自动管理
with_transaction(|_tx| {
    // ... 创建节点 ...
});
```

## 调试输出

示例中使用 `dyxel_view::println()` 输出调试信息：
- Web: 输出到浏览器控制台
- Native: 输出到 stdout

查看控制台日志可观察 Shadow Layout 与 Registry Layout 的差异。
