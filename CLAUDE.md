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
- `crates/dyxel-render-*`: 渲染后端实现。
- `sample/`: Guest 端业务逻辑示例。
- `mac/`, `android/`, `web/`: 各平台宿主入口。

## 5. 开发者契约 (Conventions)
- **性能敏感性**: 渲染循环 (Render Loop) 内禁止非必要的堆内存分配 (`Box`, `Vec`, `String`)。
- **类型安全**: 所有的 `OpCode` 必须在 `dyxel-shared/src/protocol.rs` 中集中管理，确保 Guest 和 Host 的指令解析完全对齐。
- **跨平台兼容性**: 在修改 `dyxel-shared` 或 `dyxel-render-api` 时，必须考虑到 WASM 端的单线程限制与 Native 端的并发特性。

## 6. 常见疑难 (Troubleshooting)
- **FFI 定义冲突**: 若 C/FFI 声明出现重复，优先使用 `build.rs` 的黑名单机制，保持 Rust 侧手动声明的一致性。
- **WGPU Panic**: 确保在 Android/iOS 的生命周期事件中正确管理 `Surface` 的生命周期，避免在 Surface 销毁后进行 Draw Call。
