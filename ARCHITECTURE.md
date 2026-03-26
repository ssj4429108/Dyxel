# Dyxel 架构设计规范 (v0.1)

## 1. 项目愿景与目标
**Dyxel** 是一个追求极致性能与轻量化的跨平台动态 UI 容器。
- **核心目标**：包体积 < 2MB，支持逻辑热更新，接近原生的渲染性能。
- **技术栈**：Rust (宿主) + WASM (业务逻辑) + Wasm3 (解释器) + Vello (渲染) + Taffy (布局)。

---

## 3. 核心分层架构：Three-Thread Resilience Model
为了在移动端碎片化环境下保持极致稳定性与性能，Dyxel 采用三线程垂直解耦架构：

### A. UI Thread (Platform Native)
- **职责**：持有 Surface 所有权，监听系统事件（Touch, Resize, Lifecycle）。
- **定位**：**指挥官**。不参与计算，仅负责将 OS 信号转化为内部指令并分发。

### B. Logic Thread (WASM Thinker)
- **职责**：独占 Wasm3 运行时，驱动 WASM `tick`，处理业务逻辑，执行 Taffy 布局计算。
- **定位**：**大脑**。常驻运行，确保业务状态连续性。

### C. Render Thread (GPU Rasterizer)
- **职责**：管理 `wgpu` 资源，调用渲染后端执行绘制。
- **定位**：**画笔**。与 Surface 生命周期强绑定，执行高风险 GPU 操作。

---

## 4. 生命周期韧性：同步屏障 (Synchronous Barrier)
针对 Android `onSurfaceDestroyed` 等紧急场景，Dyxel 实现了一套强同步握手机制：
1. **指令下发**：UI 线程向渲染线程发送带 ACK 的 `Suspend` 消息并进入阻塞。
2. **GPU 清理**：渲染线程执行 `device.poll(Maintain::Wait)` 强制清空工作队列并释放 Surface。
3. **安全返回**：渲染线程回传 ACK，UI 线程解除阻塞并安全返回 OS。
这彻底杜绝了“窗口已销毁但 GPU 仍在提交”导致的驱动崩溃。

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