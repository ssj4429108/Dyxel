# 当前 `dyxel-render-vello` 架构

_最后验证：2026-05-07。_

本文描述的是当前代码中的实际架构。它有意比历史设计文档更精简，重点说明当前 runtime/backend 分层、帧生命周期、平台差异以及已知的清理边界。

## Crate 角色

`dyxel-render-vello` 是 `dyxel-render-api` 中渲染器无关 API 的 Vello + wgpu 渲染实现。

当前代码同时包含：

- 绘制后端 `backend::VelloDrawingBackend`，用于较新的双层 runtime 路径；以及
- `VelloBackend`，它仍然拥有大部分渲染 pass 编排，并同时支撑新的绘制后端和旧的直接 `RenderBackend` 兼容模块（`legacy_backend.rs`）。

`VelloDrawingBackend` 刻意保持很薄：它只验证 frame 确实来自 `WgpuRuntime`，然后把实际场景渲染委托给 `VelloBackend`。

## 高层帧生命周期

```text
platform / host
  -> WgpuRuntime::begin_frame(surface_id)
       - 选择 device / queue / surface
       - 可能立即获取 surface texture
       - 在 macOS / Android 实验路径上，可能先分配离屏 frame target
       - 返回 WgpuFrameContext
  -> VelloDrawingBackend::render(frame, package)
       - 向下转型 WgpuFrameContext
       - 调用 VelloBackend::render_to_view 或 render_with_surface_texture
       - 保存 last_submission_index，供后续 GPU-ready 等待使用
  -> WgpuRuntime::end_frame(frame)
       - 呈现直接 surface texture，或
       - 延迟获取 surface texture 并将离屏 frame blit 到 surface，或
       - 在启用时使用 Android native/AHB 实验 presenter
```

backend 渲染路径不拥有 surface 生命周期。它渲染到 runtime 提供的 view。Surface 创建、resize、suspend/resume、逐帧获取以及 present 都属于 `runtime.rs` / `frame_context.rs`。

## Runtime 与 frame context 职责

### `runtime.rs`

`WgpuRuntime` 拥有：

- `wgpu::Instance` 创建；
- adapter/device/queue 选择；
- `wgpu::Surface` 创建与配置；
- surface resize/suspend/resume；
- 逐帧 surface 获取；
- offscreen-first 呈现所需的延迟 blit pipelines；
- Android present-mode 与 native-presenter 实验开关。

Native 与 wasm 行为有意通过 `cfg` gate 拆分。macOS/Android-only 的离屏 frame-ring 状态只在这些目标上编译。Android-only 的 presenter 状态只在 Android 上编译。

### `frame_context.rs`

`WgpuFrameContext` 是 Vello/wgpu 对应的具体 `BackendFrameContext`。它携带：

- 直接渲染到 surface 时的 `surface_texture`；
- offscreen-first 渲染时的 `offscreen_texture` 和 `view`；
- cloned `Device` / `Queue` handles；
- `format`、`width`、`height`、timing 字段；
- native GPU-ready wait 使用的 `last_submission_index`；
- Android-only detached presenter 状态。

相比 native，wasm 侧的 `BackendFrameContext` trait surface 更小。当前实现用 `#[cfg(not(target_arch = "wasm32"))]` gate 住 native-only 方法（`supports_detached_present`、`present_detached`、`wait_until_gpu_ready`）。

## Backend 渲染 pass 职责

`VelloBackend` 仍然是顶层 pass coordinator，但它的字段现在已经按显式 owner struct 分组到 `state.rs` 中：

- `RendererState` — Vello renderer、pipeline cache、cold-start/loading 状态、cache metadata/deferred-init 存储、memory tier 初始化、AA 选择、scoped pipeline-cache borrowing，以及主 scene-to-texture 渲染 helper。
- `BlitState` — blit shader/pipelines、sampler、backend triple buffer，以及 fullscreen blit draw/bind-group helpers。它也拥有初始化阶段的 blit resource 存储、legacy surface-pipeline 创建/存储，以及 frame coordinator 使用的当前 triple-buffer slot accessor。
- `BlurState` — filter pipeline、blur pipelines/buffers、blur entry 生命周期、atlas/backdrop textures、blur texture pool。
- `RasterCacheState` — backend GPU raster-cache lookup、面向 traversal 的 cached draw lookup/emission adapter、bake-plan execution shell，以及 GPU texture pool。
- `ShadowCacheState` — shadow cache entries/stats、renderer-id invalidation marker，以及 shadow drawing adapter。
- `TextCacheState` — glyph-run cache/stats，以及 prepared-text drawing adapter。
- `DiagnosticsState` — perf monitor、overlay toggles、begin-frame accounting、frame timing、scheduler perf stats，以及 diagnostic logging accessors。

这主要还是结构性拆分：多个低耦合 lifecycle helper 现在位于各自的 state owner 上（`RendererState`、`BlitState`、`BlurState`、`RasterCacheState`、`ShadowCacheState`、`TextCacheState`、`DiagnosticsState`）。`BlurState` 还拥有 blur scene-entry 生命周期管理、pass-2 source processing、pass-3 deferred-children rendering、pass-3.5 atlas packing，以及 pass-4 blur composite command emission；而 `VelloBackend` 仍然协调整体顺序，并拥有最终 render pass setup / 跨 state resource borrowing。

一旦调用方已经打开 render pass，实际 fullscreen blit draw 操作由 `BlitState` 拥有。当前主 pass 顺序如下：

1. 每帧 housekeeping：
   - 收集 returned textures；
   - reset blur staging；
   - lazy/cold renderer 初始化检查。
2. Scene 构建：
   - 从 immutable `RenderPackage` 创建 Vello `Scene`；
   - 通过 `scene_renderer.rs` 遍历节点并绘制内容；
   - 收集 raster-cache draw commands；
   - 遍历节点时 enqueue blur entries，并由 `BlurState` 管理 blur-entry 生命周期。
3. Raster cache baking：
   - `RasterCacheState` 执行 core/runtime 提供的 bake/recycle plans；
   - `VelloBackend` 提供递归 scene-building callback，用于填充每个 bake scene；
   - `RasterCacheState` 将 GPU texture IDs 存入 backend-owned lookup state。
4. 主 scene render：
   - 请求 `RendererState` 将 scene 渲染到 backend triple-buffer texture。
5. Blur processing：
   - copy backdrop/source regions；
   - 可选使用 atlas-wide blur；
   - 为 blur entries 渲染 deferred children；
   - 将 entries pack 到 blur atlas；
   - 将 blur composite 到最终 target 上。
6. 最终 blit：
   - 在 runtime target view 上打开最终 render pass；
   - 请求 `BlitState` blit 主 triple-buffer texture；
   - composite cached draws 和 blur entries；
   - submit GPU work，并返回 submission index。

## Scene traversal

`scene_renderer.rs` 拥有 `VelloBackend` 的递归节点遍历与节点绘制逻辑。它处理：

- 遍历期间通过 `RasterCacheLookup` 进行 raster-cache lookup；
- 通过 `ShadowCacheState` 委托 shadow draw；
- 遍历 blur nodes 时创建 blur-entry；
- 通过 `TextCacheState` 绘制文本，以及矩形绘制；
- 递归 child traversal 与 layer push/pop 行为。

目前这是文件级拆分，而不是新的 state owner，因为 traversal 有意跨越 shadow、text、blur 与 raster-cache 关注点。内部的 `TraversalContext` 组合了长生命周期 traversal inputs 与 mutable outputs，而 `NodeGeometry` / `LayerDecision` 组合了单节点的 position/size/transform 和 layer/blur/cache-subtree decisions。Leaf drawing steps 被拆成模块内的小 helper（`draw_shadow_if_needed`、`apply_blur_if_needed`、`draw_node_content`、cached-draw emission、child recursion，以及 layer push helper），让递归 traversal 主体专注于顺序。Shadow/text cache internals 保持在各自 state owner 方法之后，而不是被 traversal 直接访问；raster-cache lookups 则通过 `RasterCacheLookup` adapter 路由。

## Blur pipeline

Blur 代码已部分拆分到 `blur/`，blur resources 被分组到 `BlurState` 下。Scene-entry 生命周期管理、pass-2 blur source processing、pass-3 deferred-children rendering、pass-3.5 atlas packing，以及 pass-4 blur composite command emission 现在都位于 `BlurState` 上。最终 pass-4 render-pass setup 仍由 `VelloBackend` 协调，因此还不是完整的 ownership split。

当前重要模块：

- `blur/types.rs` — blur entry 与 texture 数据类型。
- `blur/entry.rs` — entry 构建与坐标 helper。
- `blur/passes.rs` — pass-level blur helper。
- `blur/atlas.rs` 和 `blur/atlas_pass.rs` — atlas layout 与 packing。
- `blur/children.rs` — `BlurState` 针对 blur entries 的 pass-3 deferred child rendering。
- `blur/composite.rs` — 最终 blur compositing command emission。
- `blur/pipeline.rs` — blur pipeline/texture/buffer setup，以及当前 `BlurState` pass-2 与 atlas-packing 编排。
- `filter_pipeline.rs` — blur passes 使用的 Kawase/filter 实现。

当前 blur state ownership 被分组在 `VelloBackend::blur_state` 字段下，例如：

- `filter_pipeline`；
- `blur_composite_pipeline` 和 bind-group layouts；
- `blur_instanced_*` buffers/pipelines；
- `blurred_textures`；
- `backdrop_blur`；
- `blur_atlas` / `blur_source_atlas`；
- `texture_pool`。

未来的行为保持拆分应继续降低剩余的 `VelloBackend` blur coordination surface，同时让最终 wgpu render pass lifetime 继续由顶层 frame coordinator 拥有。

## Cache 生命周期

### Raster cache

Runtime/core 通过 `RenderPackage::bake_plans` 决定要 bake 什么。Vello backend 执行 bakes，并维护 GPU-local lookup state：

- `cached_textures`：node ID 到 raster-cache texture ID；
- `gpu_texture_pool`：GPU texture 分配/复用；
- `cache::CachedDraw`：cached subtrees 的 draw commands。

`RasterCacheState` 拥有 recycle/acquire/record helpers、scoped cached-texture lookup access（`RasterCacheLookup`）、cached-draw emission、GPU texture-pool installation/borrowing，以及 bake-plan loop。`VelloBackend` 仍然为每次 bake 提供递归 scene-rendering callback，因为节点 traversal 会使用 raster-cache ownership 之外的 shadow/text/blur rendering helpers。

### Shadow cache

`shadow.rs` 拥有 shadow cache key/entry 类型与 draw helper。Entries 按量化后的 geometry/style 作为 key，并在 stale 时 evict。backend 还会限制每帧 shadow cache misses，避免 cold-frame GPU submit spike。

### Text cache

`text.rs` 按 font、size、color 与 glyph signature 缓存 prepared glyph runs。它避免每帧为静态文本重建 Vello glyph arrays，并带有简单的 size 与 LRU-style 约束。

### Pipeline/shader cache

Cold-start 与 staged initialization 被拆到：

- `cold_start.rs`；
- `minimal_shaders.rs`；
- `shader_cache.rs`；
- `staged_init.rs`；
- `staged_loader.rs`；
- `two_stage_init.rs`。

`RendererState` 组合 renderer handle、pipeline cache、cache path、cache stage、loading flag、loading thread handle，以及用于 cache metadata、deferred init、scoped pipeline-cache borrowing 和 memory optimizer setup 的小型初始化 helper。`VelloBackend` 仍然协调 renderer 初始化何时开始，以及何时消费初始化完成的 renderer。

## 平台特定路径

### wasm32

- `SharedMutex<T>` 在 `dyxel-render-api` 中是 `RefCell<T>`。
- 对 `SharedMutex` 调用 `.lock().unwrap()` 的模块必须在 `#[cfg(target_arch = "wasm32")]` 下导入 `LockExt`。
- 不应关心 native/wasm 具体类型的跨平台调用点可以使用 `dyxel_render_api::lock_shared` / `try_lock_shared`，它们返回平台无关的 `SharedMutexGuard`。
- Native-only frame context 方法不是 wasm `BackendFrameContext` trait surface 的一部分。
- Android/macOS offscreen presenter 状态不会为 wasm 编译。

### macOS

- `WgpuRuntime::begin_frame` 可以使用 offscreen-first rendering，避免在 backend 路径中阻塞等待 drawable acquisition。
- `end_frame` 会延迟获取 surface texture，并将 offscreen frame blit 到 surface。

### Android

Android 当前有若干 experimental/diagnostic 路径：

- present mode override（`DYXEL_ANDROID_PRESENT_MODE`）；
- max frame latency override；
- optional opaque surface mode；
- optional full-frame offscreen path；
- optional offscreen copy present；
- optional surface-ready wait；
- Android native presenter / SurfaceControl / AHardwareBuffer probes；
- wgpu AHB texture/frame experiments。

这些路径有意通过 cfg gate 隔离，并且除非通过环境变量显式启用，否则应保持默认关闭。

## Runtime 配置来源

Runtime 行为当前由多个模块读取环境变量控制：

- `runtime.rs`：Android present/acquire/offscreen 行为；
- `frame_context.rs`：detached blit wait 行为；
- `android_native_presenter.rs` 和 `android_native_wgpu_ahb.rs`：native presenter probes；
- `debug_utils.rs`：debug output location。

这套方式可工作，但分布较散。未来应使用 `RuntimeConfig` / `AndroidRuntimeConfig` / `DiagnosticsConfig` 集中一次性读取配置。只有那些有意在运行时动态切换的开关才应保留动态读取。

## Test-only experimental modules

以下早期 layer/offscreen experiments 已移动到显式 test-only module 下：

- `crates/dyxel-render-vello/src/experimental/composite_pipeline.rs`
- `crates/dyxel-render-vello/src/experimental/layer.rs`

`lib.rs` 只在 `#[cfg(test)]` 下挂载 `experimental`，因此这些文件不是生产 backend 路径的一部分。它们的 unit tests 与 type checks 仍会随 `cargo test -p dyxel-render-vello --lib` 运行，既能避免 silent bitrot，也能明确其 experimental 状态。

`offscreen_renderer.rs` 也属于这一类别，并且已作为 dead code 删除，符合历史 offscreen-cleanup 计划。保留 tracked 但未编译的模块会增加架构歧义。

## 当前已知架构债

- `VelloBackend` 现在已有显式 state-owner groups，但它仍然是较大的 pass coordinator，而不是薄 dispatcher。
- 仍有多个文件实现 `impl VelloBackend`；现在字段已经分组，许多函数应继续移动到相关 state owner 后面。
- `scene_renderer.rs` 仍然将 traversal 与 shadow/text/blur/raster-cache 渲染细节耦合在一起。
- `filter_pipeline.rs`、`runtime.rs`、`lib.rs` 和 `android_native_presenter.rs` 仍然较大。
- Android 配置读取仍分散在多个模块。
- `SharedMutex` 可以跨平台工作，但许多现有模块仍调用 `.lock().unwrap()`，因此必须记得 wasm `LockExt` import。新的跨平台 helper 在不需要具体 mutex 类型时，应优先使用 `lock_shared` / `try_lock_shared`。

## 推荐的下一步行为保持改造

1. 在不改变算法的前提下，继续把方法移动到新的 state owners 后面：
   - `BlurState` 已拥有 scene-entry 生命周期以及 process/deferred-children/pack/composite helpers；剩余 blur orchestration 边界是顶层 final render-pass lifetime 与跨 state resource borrowing。
   - `BlitState` 已拥有 pipeline setup、triple-buffer lifecycle、texture bind-group creation 和 fullscreen draw helpers；剩余边界是 caller-owned render pass 与 target selection。
   - `RasterCacheState` 拥有 lookup/emission helpers、GPU texture-pool access 与 bake-plan shell；剩余 raster-cache 边界是将递归 bake-scene construction 与 main scene renderer 进一步分离。
   - `scene_renderer.rs` 现在有内部 `TraversalContext`、`NodeGeometry`、`LayerDecision` 和 leaf drawing helpers；shadow/text cache access 通过 state-owner drawing methods 路由，raster-cache lookup 使用 `RasterCacheLookup`。最近的 cleanup 也已将 blit、renderer initialization 与 diagnostics 路径放到 state-owner accessors 后面。下一步安全改造可以是把更多 cold-start orchestration 移到 `RendererState` helpers 后面，或者拆分 `runtime.rs` / Android presenter 职责。
2. 按职责将 Android native presenter 移到子模块：
   - properties/env；
   - dynamically loaded Android symbols；
   - SurfaceControl；
   - AHardwareBuffer lifecycle；
   - Vulkan import；
   - sync fd；
   - probes；
   - presenter orchestration。
3. 集中构建 runtime configuration。
4. 每一步之后保持验证矩阵为绿：

```bash
cargo check --workspace
cargo check -p dyxel-web --target wasm32-unknown-unknown
cargo check -p dyxel-render-vello --target wasm32-unknown-unknown
cargo test -p dyxel-render-vello --lib
cargo fmt -- --check
git diff --check
```
