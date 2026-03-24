# Dyxel 架构设计规范 (v0.1)

## 1. 项目愿景与目标
**Dyxel** 是一个追求极致性能与轻量化的跨平台动态 UI 容器。
- **核心目标**：包体积 < 2MB，支持逻辑热更新，接近原生的渲染性能。
- **技术栈**：Rust (宿主) + WASM (业务逻辑) + Wasm3 (解释器) + Vello (渲染) + Taffy (布局)。

---

## 2. 核心分层架构：Thin Guest, Thick Host
为了在解释执行环境下保持高性能，Dyxel 遵循“**瘦客户端，胖宿主**”原则。

### A. Host 宿主层 (Rust)
- **职责**：持有渲染上下文（Vello）、布局引擎（Taffy）、原生系统事件监听、内存管理。
- **性能要求**：禁止在渲染循环中进行非必要的堆分配，利用指令缓冲区进行批量操作。
- **接口**：通过 `Wasm3` 暴露原生函数（Host Functions）供 Guest 调用。

### B. Guest 逻辑层 (WASM)
- **职责**：业务状态管理、UI 声明（类似声明式 UI）、交互逻辑处理。
- **运行环境**：Wasm3 解释器（无 JIT 模式，以完美适配 iOS 和 Android 的合规性要求）。
- **通信**：通过自定义的 **Binary Instruction Stream** 向 Host 发送 UI 变更意图。

---

## 3. 关键技术协议

### 3.1 指令流 (Instruction Stream)
Guest 不直接操作渲染器，而是向共享内存写入指令序列。
- **格式**：固定长度的指令头（OpCode） + 可变长度的操作数（Payload）。
- **对齐**：所有数据交换必须符合 4-byte 对齐，以优化跨语言内存读取性能。
- **同步机制**：采用稀疏同步（Sparse Sync），仅在属性发生变化时发送变更指令。

### 3.2 布局系统 (Layout)
- 宿主侧使用 **Taffy** 实现高性能 Flexbox/Grid 布局。
- WASM 层仅维护虚拟 Node ID 和 Style 属性，Host 负责最终的几何空间计算（$x, y, width, height$）。

### 3.3 渲染流水线 (Pipeline)
1. **Logic Tick**: WASM 执行业务逻辑，更新内部状态。
2. **Commit**: WASM 将变更指令打包写入 Ring Buffer。
3. **Host Process**: Rust 宿主读取指令，同步更新 Taffy 树并触发布局计算。
4. **Render**: Vello 接收布局结果，将其转换为 GPU 高速渲染指令。

---

## 4. 编码原则 (Aider 必须遵守)
1. **安全优先**：尽量避免使用 `unsafe` 块；若必须使用，须附带详尽的 `// SAFETY:` 论证。
2. **零拷贝 (Zero-Copy)**：在 WASM 和 Rust 交换大数据（如 Image Data 或 Path Data）时，必须通过 `Linear Memory` 视图直接操作，严禁在 FFI 边界进行全量 `Clone`。
3. **平台无关性**：核心引擎代码严禁直接绑定 `ANativeWindow` 或 `UIView`，必须通过抽象层进行平台解耦。
4. **模块化设计**：确保渲染器后端（Renderer Backend）可插拔，以便未来支持不同的 GPU 后端。

---

## 5. 待办开发优先级 (Roadmap)