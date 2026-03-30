# Dyxel 项目全局规范 (Global Standards)

## 1. 项目愿景 (Vision)
**Dyxel**: 超轻量 (<2MB)、逻辑热更新、高性能渲染 (Vello/Impeller) 的跨平台 UI 容器。
遵循 "**Thin Guest, Thick Host**" 原则：Host 负责渲染和布局 (Taffy)，Guest 负责交互逻辑。

## 2. 架构规范 (Architecture)
- **FFI 边界**: 严禁在 Guest (WASM) 和 Host 之间频繁传递大型结构体。必须通过 `dyxel-shared` 定义的二进制指令流进行同步。
- **渲染解耦**: 核心逻辑不得依赖具体的 GPU 后端（WGPU, Metal, Vulkan），必须通过 `RenderBackend` 接口操作。
- **并发策略**: 
  - 非 WASM 环境：使用 `Arc<Mutex<T>>`。
  - WASM 环境：使用 `Rc<RefCell<T>>`。
  - 必须通过抽象类型名在跨平台代码中引用，以确保一套代码兼容多端。

## 3. 开发命令 (Commands)
- **全平台构建**: `./check_all.sh` (核心检查点)
- **macOS (开发主力)**: `./build_mac.sh`
- **Web (WASM 调试)**: `./build_web.sh`
- **Android**: `./build_android.sh`
- **iOS**: `./build_ios.sh`
- **格式化**: `cargo fmt --all`

## 4. 关键路径说明 (Codebase Map)
- `crates/dyxel-core`: 宿主核心，包含 WASM 运行时和 Bridge 逻辑。
- `crates/dyxel-shared`: 协议层，定义了 Guest/Host 的内存布局和指令集。
- `crates/dyxel-view`: WASM 端视图库，包含 Shadow Layout 实现。
- `crates/dyxel-render-*`: 渲染后端实现。
- `sample/`: Guest 端业务逻辑示例。
- `mac/`, `android/`, `web/`: 各平台宿主入口。

## 5. Shadow Layout 系统 (Week 3)

### 5.1 三层布局架构
为消除布局查询的 Host 往返延迟，实现三层渐进式布局系统：

| 层级 | 延迟 | API | 使用场景 |
|------|------|-----|----------|
| **Shadow** | 0ms | `get_layout_estimated(id)` | 立即响应的估算布局 |
| **Registry** | ~16ms | `get_layout(id)` / `take_layout(id)` | 已提交的 Host 布局 |
| **Host Sync** | 可变 | `force_layout()` | 强制完整布局计算 |

### 5.2 核心组件
- **ShadowTree** (`dyxel-view/src/shadow_layout.rs`): WASM 端 TaffyTree 封装
- **自动同步**: `dyxel_view_tick()` 自动同步命令流并重新计算布局
- **视口管理**: `dyxel_set_viewport_size()` Host 调用以更新视口大小

### 5.3 使用示例
```rust
// 初始化（在 WASM 启动时调用）
init_shadow_tree();

// 创建视图（自动同步到 ShadowTree）
let container = View::new()
    .width(400.px())
    .height(600.px())
    .flex_direction(FlexDirection::Column);

// 零延迟布局查询
let layout = get_layout_estimated(container.id);
println!("尺寸: {}x{}", layout.width, layout.height);

// 检查文本是否溢出
if would_text_overflow(text_node.id, text_width) {
    // 调整字体大小或截断文本
}

// 获取底部位置（瀑布流布局）
let bottom_y = get_estimated_bottom_y(item.id);
```

### 5.4 实现细节
- **依赖**: Taffy 0.9 (`default-features = false, features = ["flexbox"]`)
- **存储**: `thread_local! { RefCell<Option<ShadowTree>> }` (WASM 单线程)
- **同步**: 命令流增量同步，`process_command_batch()` 处理新命令
- **测试**: 23 个单元测试覆盖核心功能

## 6. 开发者契约 (Conventions)
- **性能敏感性**: 渲染循环 (Render Loop) 内禁止非必要的堆内存分配 (`Box`, `Vec`, `String`)。
- **类型安全**: 所有的 `OpCode` 必须在 `dyxel-shared/src/protocol.rs` 中集中管理，确保 Guest 和 Host 的指令解析完全对齐。
- **跨平台兼容性**: 在修改 `dyxel-shared` 或 `dyxel-render-api` 时，必须考虑到 WASM 端的单线程限制与 Native 端的并发特性。
- **Shadow Layout 原则**: ShadowTree 是估算而非精确值，复杂场景（文本测量）仍需等待 Host 布局。

## 7. 常见疑难 (Troubleshooting)
- **FFI 定义冲突**: 若 C/FFI 声明出现重复，优先使用 `build.rs` 的黑名单机制，保持 Rust 侧手动声明的一致性。
- **WGPU Panic**: 确保在 Android/iOS 的生命周期事件中正确管理 `Surface` 的生命周期，避免在 Surface 销毁后进行 Draw Call。
- **Shadow Layout 不同步**: 确保 `dyxel_view_tick()` 被定期调用（每帧一次），以保证 ShadowTree 与命令流同步。
