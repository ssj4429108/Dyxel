# 🎯 最终 Demo - Counter RSX

完整集成：`#[app]` 宏 + `rsx!` 宏 + 状态管理

## 📁 文件结构

```
sample/src/
├── lib.rs              # 入口（使用 #[app] 宏，无需手动导出）
└── counter_rsx.rs      # 完整业务模块
```

## 🎨 功能特性

| 功能 | 实现 |
|-----|------|
| 状态管理 | `use_state` 管理计数和消息 |
| 计算属性 | `use_memo` 动态计算颜色和显示文本 |
| 响应式布局 | Flex 布局，居中显示 |
| 交互反馈 | 点击按钮更新状态和消息 |
| 视觉反馈 | 背景色根据计数自动变化 |

## 📝 核心代码

```rust
use dyxel_app::prelude::*;

#[app]
pub fn CounterApp() -> impl BaseView {
    // ===== 状态管理 =====
    let count = use_state(|| 0);
    let message = use_state(|| "点击按钮开始计数".to_string());
    
    // 计算属性：颜色根据 count 变化
    let bg_color = use_memo({
        let count = count.clone();
        move || {
            match count.get() {
                c if c < 0 => (180, 50, 50),    // 负数：红色
                0 => (50, 50, 50),               // 零：深灰
                c if c < 5 => (50, 100, 50),     // 小正数：绿色
                _ => (50, 50, 180),              // 大数：蓝色
            }
        }
    });
    
    // ===== UI 构建 =====
    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: bg_color,           // 动态颜色
            flexDirection: FlexDirection::Column,
            justifyContent: JustifyContent::Center,
            alignItems: AlignItems::Center,
            
            // 标题
            Text("🎯 计数器 Demo") {
                fontSize: 32.0,
            }
            
            // 计数显示卡片
            View {
                width: 200.0,
                height: 120.0,
                color: (255, 255, 255),
                borderRadius: 16.0,
                
                Text(format!("当前计数: {}", count.get())) {
                    fontSize: 36.0,
                }
            }
            
            // 按钮区域
            View {
                flexDirection: FlexDirection::Row,
                
                Button {
                    color: (220, 80, 80),
                    onTap: move || {
                        count.set(count.get() - 1);     // 更新状态
                        message.set("减少了！".to_string());
                    },
                    Text("−") { fontSize: 28.0 }
                }
                
                Button {
                    color: (80, 180, 80),
                    onTap: move || {
                        count.set(count.get() + 1);     // 更新状态
                        message.set("增加了！".to_string());
                    },
                    Text("+") { fontSize: 28.0 }
                }
            }
        }
    }
}
```

## 🚀 使用方式

### lib.rs（入口）
```rust
mod counter_rsx;  // 仅此一行！

// #[app] 宏自动生成 main() 和 guest_tick()
```

### 构建
```bash
./build_android.sh && cd android && ./gradlew assembleDebug
```

## 📚 关键技术点

1. **`#[app]` 宏**：标记入口函数，自动生成 WASM 导出
2. **`rsx!` 宏**：声明式 UI，类似 JSX
3. **`use_state`**：响应式状态，自动触发重渲染
4. **`use_memo`**：计算属性，依赖变化时自动更新
5. **`onTap`**：手势事件处理

## ✅ 预期效果

- 深灰色背景（初始）
- 白色卡片显示计数
- 红色/重置/绿色三个按钮
- 点击 +/− 按钮增减计数
- 背景色随计数变化（负数红、零灰、小数绿、大数蓝）
