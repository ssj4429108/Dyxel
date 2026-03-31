# Dyxel 模块化开发指南

## 推荐文件结构

```
sample/src/
├── lib.rs              # 入口：声明模块 + 导出函数
├── counter_simple.rs   # 业务模块 A：#[app] 宏版本
├── counter_manual.rs   # 业务模块 B：手动导出
├── todo_app.rs         # 业务模块 C：完整应用
└── components/         # 公共组件（可选）
    ├── mod.rs
    ├── header.rs
    └── button.rs
```

## 方式一：#[app] 宏（简单场景）

### counter_simple.rs
```rust
use dyxel_app::prelude::*;

#[app]
pub fn Counter() -> impl BaseView {
    let count = use_state(|| 0);
    
    View::new()
        .width("100%")
        .height("100%")
        .color((30, 30, 30))
        .child(
            Text::new()
                .value(format!("Count: {}", count.get()))
                .font_size(48.0)
                .node_id()
        )
}
```

### lib.rs
```rust
mod counter_simple;  // 引入模块，宏自动生成导出

// 不需要手动写 main() 和 guest_tick()！
```

## 方式二：手动导出（复杂场景）

### counter_manual.rs
```rust
use dyxel_view::{View, Text, BaseView, force_layout};
use dyxel_state::use_state;

pub fn init() {
    let count = use_state(|| 0);
    
    let root = View::new()
        .width("100%")
        .height("100%")
        .color((30, 30, 30));
    
    let text = Text::new()
        .value(format!("Count: {}", count.get()))
        .font_size(48.0);
    
    BaseView::child(root, text.node_id());
    force_layout();
}

pub fn tick() {
    dyxel_view::dyxel_view_tick();
}
```

### lib.rs
```rust
mod counter_manual;  // 引入模块

#[unsafe(no_mangle)]
pub extern "C" fn main() {
    counter_manual::init();
}

#[unsafe(no_mangle)]
pub extern "C" fn guest_tick() {
    counter_manual::tick();
}
```

## 对比

| 特性 | #[app] 宏 | 手动导出 |
|-----|-----------|----------|
| 代码量 | 少 | 多 |
| 灵活性 | 受限 | 完全控制 |
| 调试难度 | 需理解宏 | 直接可见 |
| 适用场景 | 简单页面 | 复杂应用 |

## 最佳实践

1. **简单页面**：用 `#[app]` 宏，快速开发
2. **复杂应用**：手动导出，精细控制
3. **多页面应用**：每个页面对应一个模块，在 lib.rs 中切换
4. **公共组件**：放在 `components/` 目录，供多个模块复用
