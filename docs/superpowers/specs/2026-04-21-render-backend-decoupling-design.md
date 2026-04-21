# 渲染后端极度解耦与可插拔接入设计

## 1. 背景

当前渲染编排已经基本收敛为 scheduler-centric：

- `FrameScheduler` 负责 frame ownership
- `Logic Worker` 负责生产最新 `RenderPackage`
- `Render Worker` 负责消费 token + mailbox 并执行渲染

这一层对具体后端已经相对中立。

但在图形运行时和平台接缝层，系统仍然明显偏向 `Vello + wgpu`：

- `dyxel-core` 仍直接创建 `VelloBackend`
- `dyxel-core` 和平台层仍直接操作 `vello::util::RenderContext`
- `mac/android/web` 仍直接构造 `wgpu::SurfaceTarget`
- `render-api` 虽然已有 backend trait，但 `RenderContext` / `SurfaceTargetHandle` 仍主要依赖 `Any` + downcast

这意味着当前架构虽有 backend abstraction 雏形，但还不能低成本切换到 `Impeller` 或未来的 `Skia`。

系统需要一次明确的架构性收口，使以下目标成立：

- `scheduler/core` 不感知后端
- `platform` 不感知后端
- 编译期可以选择 `Vello / Impeller / Skia`
- `RenderPackage` 在可行范围内复用

## 2. 目标

本设计目标如下：

1. 建立适用于 `Vello / Impeller / Skia` 的可插拔 backend 接入模型
2. 让 `dyxel-core` 只依赖中立 render API，不再直接依赖具体 backend crate
3. 让平台层只上报 native surface/window handle，不再构造 backend-specific surface 对象
4. 将“图形运行时接缝”和“具体渲染引擎”拆成两层
5. 保持现有 scheduler/mailbox/render token 编排语义不变
6. 在不强制统一所有底层绘制机制的前提下，尽量复用 `RenderPackage`

## 3. 非目标

本设计明确不包含以下内容：

1. 不在本轮强行统一所有 backend 的底层绘制实现  
   文字、阴影、模糊、滤镜等具体落地方式允许 backend 自己适配。

2. 不要求 `RenderPackage` 一开始就是“完美统一渲染 IR”  
   本轮优先建立可插拔边界，再逐步中立化公共数据结构。

3. 不改动 scheduler 的 frame contract  
   cadence、mailbox、frame token、timeline 的语义应保持不变。

4. 不把平台主循环逻辑搬入 backend  
   平台仍负责生命周期和 native handle 提供，只是不再理解后端图形对象。

5. 不在本轮直接完成 `Impeller` 或 `Skia` 的完整生产级实现  
   本设计先解决接入模型和架构边界。

## 4. 总体思路

现有单层 `RenderBackend` 模型不足以承载多 backend，因为它同时混合了两类职责：

- 平台图形运行时接缝
- 具体渲染引擎绘制实现

本设计改用双层模型：

- `GraphicsRuntime`
- `RenderBackend`

### 4.1 GraphicsRuntime

`GraphicsRuntime` 负责平台和图形运行时接缝，不负责绘制内容。

职责包括：

- 创建和持有图形 runtime 上下文
- 从 native handle 创建 surface / swapchain
- surface resize / suspend / resume / sync / present
- 处理主线程 surface 创建等平台约束
- 为 backend 提供当帧可渲染上下文

### 4.2 RenderBackend

`RenderBackend` 负责消费 `RenderPackage` 并执行具体绘制。

职责包括：

- backend 自身初始化
- scene draw / text / blur / shadow / layer 等具体实现
- raster cache bake / recycle 的执行
- backend-specific overlay / warmup / timing 等能力

### 4.3 边界关系

目标依赖关系如下：

- `platform -> dyxel-render-api`
- `dyxel-core -> dyxel-render-api`
- `dyxel-render-vello -> dyxel-render-api`
- `dyxel-render-impeller -> dyxel-render-api`
- `dyxel-render-skia -> dyxel-render-api`

其中：

- `platform` 只提供 `NativeSurfaceHandle`
- `dyxel-core` 只协调 runtime + backend
- runtime 决定如何解释 native handle
- backend 决定如何消费 `RenderPackage`

## 5. 目标架构

### 5.1 编译期切换模型

系统采用编译期选择 backend 的模型。

即：

- 一次构建只启用一个 backend 组合
- 但所有 backend 都遵守同一套 `dyxel-render-api`

示例：

- `Vello = WgpuRuntime + VelloBackend`
- `Impeller = ImpellerRuntime + ImpellerBackend`
- `Skia = SkiaRuntime + SkiaBackend`

### 5.2 Core 不感知后端

`dyxel-core` 不再：

- `use dyxel_render_vello::VelloBackend`
- downcast `vello::util::RenderContext`
- downcast `vello::wgpu::Instance`
- 构造 `wgpu::SurfaceTarget`

它只依赖：

- `GraphicsRuntimeFactory`
- `GraphicsRuntime`
- `RenderBackend`
- `BackendCapabilities`

### 5.3 Platform 不感知后端

`mac/android/web` 不再直接创建 backend-specific surface target。

平台层只提供：

- 原生窗口句柄
- 原生 surface 句柄
- canvas 句柄

然后通过中立接口传给 runtime。

## 6. 接口模型

### 6.1 GraphicsRuntimeFactory

编译期选定 backend 后，`dyxel-core` 通过 factory 创建 runtime 和 backend。

建议接口：

```rust
pub trait GraphicsRuntimeFactory: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn capabilities(&self) -> BackendCapabilities;

    fn create_runtime(&self) -> anyhow::Result<Box<dyn GraphicsRuntime>>;
    fn create_backend(&self) -> anyhow::Result<Box<dyn RenderBackend>>;
}
```

### 6.2 BackendCapabilities

用纯数据表达 backend 能力，不再靠 downcast 判断。

建议结构：

```rust
pub struct BackendCapabilities {
    pub perf_overlay: bool,
    pub gpu_timing: bool,
    pub renderer_warmup: bool,
    pub main_thread_surface_creation: bool,
    pub main_thread_rendering: bool,
    pub explicit_present: bool,
}
```

### 6.3 NativeSurfaceHandle

平台层只传 native handle，不传 backend-specific surface 对象。

建议优先兼容 `raw-window-handle` 社区标准，而不是在公共 API 中手写过多平台专用分支。

建议结构：

```rust
pub enum NativeSurfaceHandle {
    RawWindow {
        window: raw_window_handle::RawWindowHandle,
        display: raw_window_handle::RawDisplayHandle,
    },
    #[cfg(target_arch = "wasm32")]
    WebCanvas {
        canvas_id: String,
    },
    NativeSurface {
        kind: NativeSurfaceKind,
        ptr: u64,
    },
}
```

其中：

- `RawWindow` 作为桌面平台的默认路径
- `WebCanvas` 作为 Web 的特化路径
- `NativeSurface` 作为 Android `ANativeWindow*` 等无法自然映射到 `raw-window-handle` 的兜底路径

原则是：

- 平台层只上报 Rust 社区标准句柄或原生句柄
- runtime 负责将其转换成具体图形 API 需要的 surface 对象

### 6.4 GraphicsRuntime

`GraphicsRuntime` 负责 surface 与 runtime 生命周期，不负责 scene 绘制。

建议最小接口：

```rust
pub trait GraphicsRuntime: Send + Sync {
    fn initialize(&mut self) -> anyhow::Result<()>;

    fn create_surface(
        &mut self,
        handle: NativeSurfaceHandle,
        width: u32,
        height: u32,
    ) -> anyhow::Result<RuntimeSurfaceId>;

    fn resize_surface(
        &mut self,
        surface: RuntimeSurfaceId,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()>;

    fn suspend(&mut self) -> anyhow::Result<()>;
    fn resume(&mut self) -> anyhow::Result<()>;
    fn sync_gpu(&mut self) -> anyhow::Result<()>;

    fn begin_frame(
        &mut self,
        surface: RuntimeSurfaceId,
    ) -> anyhow::Result<Box<dyn BackendFrameContext>>;

    fn end_frame(
        &mut self,
        frame: Box<dyn BackendFrameContext>,
    ) -> anyhow::Result<()>;
}
```

`begin_frame()` 与 `end_frame()` 明确了 frame submission ownership：

- runtime 创建当帧上下文
- backend 只在该上下文上编码和执行绘制
- present / swapbuffers / submit 的最终归属由 runtime 控制

这样可以兼容：

- `wgpu` 风格的显式 submit/present
- Impeller 风格的 surface present
- 未来对线程有严格要求的 backend

### 6.5 BackendFrameContext

当帧上下文由 runtime 创建并交给 backend 消费。

短期允许内部使用 `Any`，但 downcast 只允许发生在 runtime/backend 之间，不再泄漏到 core/platform。

为避免 backend 意外消费错误 runtime 产出的 context，需要增加 runtime/backend 兼容性校验。

建议接口：

```rust
pub trait BackendFrameContext {
    fn as_any(&mut self) -> &mut dyn std::any::Any;
    fn runtime_kind(&self) -> RuntimeKind;
}
```

并要求：

- `GraphicsRuntimeFactory` 创建的 runtime 与 backend 必须来自同一 backend 族
- `RenderBackend::initialize()` 时校验 `RuntimeKind`
- `render()` 若发现 runtime/backend 不匹配，必须立即返回错误，而不是尝试继续 downcast

### 6.6 RenderBackend

新 `RenderBackend` 不再承担平台 bootstrap，只负责绘制。

建议接口：

```rust
pub trait RenderBackend: Send + Sync {
    fn initialize(&mut self, runtime: &mut dyn GraphicsRuntime) -> anyhow::Result<()>;

    fn render(
        &mut self,
        frame: &mut dyn BackendFrameContext,
        package: &RenderPackage,
    ) -> anyhow::Result<RenderFrameStats>;

    fn on_lifecycle_event(&mut self, event: LifecycleEvent) -> anyhow::Result<()> {
        Ok(())
    }
}
```

### 6.7 RenderFrameStats

backend 向 scheduler/perf 系统回报中立的 frame timing 数据。

建议结构：

```rust
pub struct RenderFrameStats {
    pub cpu_time_ms: Option<f64>,
    pub gpu_time_ms: Option<f64>,
    pub backend_internal_stats: Option<serde_json::Value>,
}
```

其中 `backend_internal_stats` 用于承载 backend-specific 诊断数据，例如：

- Vello 的 compute / pipeline / staging 统计
- Skia 的 batch 数、flush 次数
- Impeller 的 pass / command buffer 统计

它不参与 scheduler 核心决策，但应保留给诊断与性能调优使用。

## 7. RenderPackage 复用策略

### 7.1 总体策略

`RenderPackage` 采用：

- 中立主干
- backend 自适配

即：

- 公共层复用 scene 语义
- backend 自己适配绘制机制

不要求所有 backend 共享完全相同的底层实现。

为了同时兼容 `Vello` 的 scene 风格和 `Impeller/Skia` 更接近 display list / canvas 的执行方式，`RenderPackage` 的公共 contract 应更接近：

- 可序列化的 display list / scene instruction list

而不是某个单一 backend 的内部场景对象。

在具体实现上，建议优先采用：

- 连续内存布局的紧凑命令流

而不是高度分散的对象图。这样更利于：

- Logic Worker 顺序生成
- backend 顺序遍历与翻译
- 后续跨 backend 共享同一份逻辑输出
- 降低多后端场景下的中间分配成本

### 7.2 必须中立化的部分

以下公共数据结构必须逐步去除 `Vello/Peniko` 假设：

- 颜色
- 字体资源引用
- 文本 payload 中的公共字段
- 阴影描述
- 模糊描述
- 混合/滤镜的公共语义

原则是：

- 公共层只表达“要什么效果”
- backend 自己决定“怎么实现”

此外，`RenderPackage` 必须明确公共逻辑坐标系与裁剪语义，包括：

- 逻辑坐标空间以像素为单位
- 原点位置
- Y 轴方向
- clip rect 的包含语义
- transform 组合顺序

不同 backend 对 NDC、像素中心、半像素偏移和 Y 轴方向的差异，必须在 backend 内部吸收，不能回流到 core/runtime。

### 7.3 可直接复用的部分

以下结构应尽量保留：

- `RenderPackage`
- `SceneNode`
- `BakePlan`
- `RecyclePlan`
- `layout_epoch`
- `dirty_tracker`
- runtime 生产 package、backend 消费 package 的流程

### 7.4 允许 backend 自己适配的部分

以下部分不要求公共层在第一阶段完全统一：

- text shaping 后的具体 glyph 提交方式
- blur/shadow 的底层绘制路径
- layer/filter 的具体实现机制
- texture/resource pool 的具体资源模型

但 image / texture 资源句柄必须在公共 contract 中保持中立：

- `RenderPackage` 只引用 `ResourceId`
- backend 通过 `ResourceProvider` 或等价资源服务解析资源
- GPU 纹理上传与 backend-specific 资源缓存由 backend 自己负责

这保证：

- `RenderPackage` 不持有具体 GPU 资源
- 不同 backend 可以在自己的上下文中完成资源上传与复用

### 7.5 Raster Cache 与 ResourceId 稳定性

`BakePlan` 与 `RecyclePlan` 保留在 `RenderPackage` 中，表示 cache policy 决策仍由 runtime/core 发出，而缓存资源本体仍保存在 backend 内。

这是允许的，但需要明确两个约束：

1. `ResourceId` / `TextureId` 必须在跨帧语义上稳定
2. backend 必须保证同一逻辑资源标识在缓存未失效前可被重复命中

也就是说：

- core 决定“该 bake / recycle 什么”
- backend 决定“如何存储这些缓存资源”
- 双方通过稳定标识而不是裸 GPU 句柄协作

## 8. 迁移策略

### Step 1：引入双层接口，但保留旧路径

在 `dyxel-render-api` 中新增：

- `GraphicsRuntimeFactory`
- `GraphicsRuntime`
- `RenderBackend`
- `BackendCapabilities`
- `NativeSurfaceHandle`

旧 `RenderContext` / `SurfaceTargetHandle` / `SurfaceHandle` 暂时保留，作为过渡层。

### Step 2：把 Vello 路径拆成第一份双层实现

在 `dyxel-render-vello` 中拆出：

- `WgpuRuntime`
- `VelloBackend`

职责分离如下：

- `WgpuRuntime` 负责 instance/device/queue/surface/present/suspend/sync
- `VelloBackend` 负责 `RenderPackage` 绘制、raster cache、overlay、timing

### Step 3：core/platform 全面改走新接口

修改：

- `dyxel-core/src/engine.rs`
- `dyxel-core/src/bridge.rs`
- `dyxel-core/src/renderer.rs`
- `mac`
- `android`
- `web`

目标是：

- `dyxel-core` 不再出现 `Vello/wgpu` 具体类型
- platform 不再构造 `wgpu::SurfaceTarget`
- render 主路径只协调 runtime + backend

### Step 4：中立化 RenderPackage 公共类型，并接入第二 backend

在控制面与平台接缝稳定后，再处理：

- `render-api` 去 `peniko` 化
- `ImpellerRuntime + ImpellerBackend`
- 未来 `SkiaRuntime + SkiaBackend`

## 9. 文件边界建议

### 9.1 dyxel-render-api

职责：

- 中立接口
- 公共类型
- capability 定义
- 中立 render package contract

### 9.2 dyxel-render-vello

建议拆分：

- `runtime.rs`：`WgpuRuntime`
- `frame_context.rs`：`WgpuFrameContext`
- `backend.rs`：`VelloBackend`
- `surface.rs`：surface 管理
- `overlay.rs`：可选 overlay/diagnostics
- `lib.rs`：对外导出和 factory

### 9.3 dyxel-core

职责：

- scheduler
- mailbox
- logic/render worker orchestration
- 调用 runtime + backend 的中立接口

不得再持有 backend-specific 图形对象。

## 10. 兼容性与风险控制

### 10.1 兼容策略

为降低迁移风险，双层接口引入初期允许：

- 新旧接口并存
- runtime/backend 内部暂时使用 `Any`
- Vello 作为第一份参考实现

但这些兼容措施不得继续泄漏到 core/platform。

### 10.2 风险点

本次迁移的最高风险点是：

- surface 生命周期
- present ownership
- 主线程创建 surface 的平台约束
- 主线程或特定线程渲染约束
- suspend/resume 与 scheduler 的协作边界
- backend 对公共坐标系和 clip 语义的吸收
- backend 间资源标识稳定性

因此迁移顺序必须先收口 runtime/bootstrap，再收口 IR 中立化。

## 11. 成功标准

当满足以下条件时，可认为本次极度解耦目标达成：

1. `dyxel-core` 只依赖 `dyxel-render-api`
2. `mac/android/web` 不再构造 backend-specific surface target
3. `dyxel-core` 中不存在 `vello::*`、`wgpu::*`、`VelloBackend` 直接依赖
4. Vello 路径能够通过 `GraphicsRuntime + RenderBackend` 正常运行现有 scheduler 流程
5. 编译期可以用同一套 core/platform 接缝切换到第二 backend 实现
6. `RenderPackage` 主干被复用，backend 只在必要的机制层做自适配

## 12. 结论

本设计采用“双层模型”作为多 backend 可插拔架构的基础：

- `GraphicsRuntime` 处理平台图形运行时接缝
- `RenderBackend` 处理具体绘制实现

它的核心收益不是立即拥有多个 backend，而是先把系统从：

- `core/platform 驱动具体后端`

收敛到：

- `core/platform 只依赖中立边界`

这使得后续接入 `Impeller` 与 `Skia` 时，主要工作被限制在 backend crate 内部，而不再反向侵入 scheduler、core 和平台层。
