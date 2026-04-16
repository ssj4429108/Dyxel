# Dyxel 离屏渲染清理与优化设计文档

**日期**: 2026-04-16  
**主题**: 统一纹理池化、清理死代码、压缩渲染管线  
**范围**: P0 (池化统一) + P1 (代码瘦身) + P2 (管线压缩)

---

## 第一章：架构与三阶段拆分

### 1.1 当前问题

Dyxel 的 Android 渲染路径中存在三类技术债务：

1. **内存抖动**: `render_with_blur` 和 Children Texture 仍使用每帧 `device.create_texture`，在 TBDR 移动端导致频繁分配与隐式同步。
2. **僵尸代码**: `offscreen_renderer.rs` 是完全未被引用的死代码；`blur_cache` 是一个从未真正跑起来的空 `HashMap`。
3. **管线冗余**: Pass 3 的全屏 Children Texture 在大量场景下可以省去；Kawase Pass 5+6 虽已合并，但内部仍存在独立的 `KawaseTexturePool`。

### 1.2 三阶段执行顺序

采用 **P1 → P0 → P2** 的执行顺序（方案 C）：

- **P1 清理坟场**: 先删除死代码和僵尸结构，降低认知负载。
- **P0 统一池化**: 在干净的代码基座上，将 `KawaseTexturePool` 合并进 `SharedTexturePool`，并为 Blur Offscreen 和 Children Texture 提供池化入口。
- **P2 管线压缩**: 最后引入 Children 的“直接渲染”优化路径，减少不必要的离屏 Pass。

### 1.3 核心设计原则

- **拒绝回退**: 若池未初始化，宁可跳过渲染也不回退到 `device.create_texture`。
- **RAII 即安全**: 所有池化纹理通过 `PooledTexture` 的 `Drop` 自动归还，不依赖手动调用。
- **正确性优先于性能**: 直接渲染路径仅在变换矩阵安全（Translation + Uniform Scale）时启用，否则自动回退到离屏路径。

---

## 第二章：组件与接口设计

### 2.1 SharedTexturePool 扩展

在 `crates/dyxel-render-vello/src/texture_pool.rs` 中，为 `TexturePool` 和 `SharedTexturePool` 新增以下方法：

```rust
/// 获取用于 Frosted Glass 输入的离屏纹理（按节点 bounding box 尺寸）
pub fn acquire_blur_offscreen(&self, width: u32, height: u32) -> PooledTexture {
    self.acquire(width, height, wgpu::TextureFormat::Rgba8Unorm)
}

/// 获取用于 Deferred Children 的离屏纹理（按节点 bounding box 尺寸）
pub fn acquire_children_texture(&self, width: u32, height: u32) -> PooledTexture {
    self.acquire(width, height, wgpu::TextureFormat::Rgba8Unorm)
}
```

**关键改动**:
- `acquire_kawase_set` 的 `quarter` 尺寸已统一为 `/8`（与内部优化对齐）。
- `PooledTexture` 新增内部 `TextureView` 预缓存：第一次调用 `view()` 时创建并缓存，后续直接返回同一对象，避免 wgpu 内部重复创建 Descriptor。

### 2.2 FilterPipeline 净化

`crates/dyxel-render-vello/src/filter_pipeline.rs`:

- 保留 `kawase_output_pipeline`（Pass 5+6 合并的成果）。
- 删除内部独立的 `KawaseTexturePool` 结构及其实例字段。
- `encode_frosted_glass_kawase` 的输入改为 `KawaseTextureSet`（由调用方从 `SharedTexturePool` 获取）。
- 不再在 `FilterPipeline` 内部创建任何纹理，所有资源管理外移到 `VelloBackend`。

### 2.3 BlurredTextureEntry 所有权变更

在 `crates/dyxel-render-vello/src/lib.rs` 中：

```rust
pub struct BlurredTextureEntry {
    pub texture: PooledTexture,        // 以前是 wgpu::Texture
    pub bind_group: wgpu::BindGroup,
    pub uniform_offset: wgpu::BufferAddress,
}
```

**收益**: `BlurredTextureEntry` 被 `TripleBuffer` 的 Frame 数据持有，Frame 结束后自然 Drop，`PooledTexture` 自动归还池子，无需显式生命周期管理。

### 2.4 blur_cache 重构为真正的纹理缓存

将 `blur_cache: SharedMutex<HashMap<u32, CachedBlurResult>>` 改为基于**内容哈希**的缓存：

```rust
pub struct CachedBlurResult {
    pub content_hash: u64,             // 节点内容哈希（如子树命令流哈希）
    pub texture: PooledTexture,
    pub bind_group: wgpu::BindGroup,
    pub frame_counter: u64,            // 用于 LRU 淘汰
}
```

**缓存命中条件**: 同一节点在连续帧中 `content_hash` 不变时，直接使用 `CachedBlurResult.texture`，跳过整个 Kawase Pass。

---

## 第三章：数据流与渲染时间线

### 3.1 单帧渲染流程

```
[Frame Start]
    ↓
SharedTexturePool::collect_returns()      // 归还上一帧的 PooledTexture
    ↓
Phase 1: 评估 blur_cache 和内容哈希
    - 命中 → 复用 CachedBlurResult
    - 未命中 → 继续 Phase 2
    ↓
Phase 2: 从 SharedTexturePool 获取资源袋
    - acquire_blur_offscreen(w, h) 用于 Vello 离屏渲染
    - acquire_kawase_set(full_w, full_h) 用于 Kawase 中间纹理
    ↓
Phase 3: Vello 渲染到 blur offscreen texture
    ↓
Phase 4: FilterPipeline 录制 Kawase 命令
    - Downsample → Iterations → Output (合并后的 Pass)
    - 输出到 BlurredTextureEntry.texture (PooledTexture)
    ↓
Phase 5: 存入 blur_cache（若启用缓存）
    ↓
Phase 6: 评估 Children 渲染路径
    - children_can_direct_render == true
        → 在 Pass 4 直接利用 Vello + scissor 渲染到主场景
    - false
        → acquire_children_texture(bb_w, bb_h)
        → Vello 渲染到 Children Texture
        → Pass 4 合成
    ↓
[Frame End: 所有 PooledTexture 随 Frame 数据 Drop 归还]
```

### 3.2 关键所有权流转

1. `SharedTexturePool` 是 `VelloBackend` 的长生命周期成员。
2. 每帧开始时，调用 `collect_returns()` 处理上一帧 Drop 回来的纹理。
3. `acquire_*` 将所有权移交给当前 Frame。
4. Frame 结束后，`PooledTexture` 的 `Drop` 通过 mpsc 将纹理发回池子，等待下一帧 `collect_returns()` 回收。

### 3.3 Children 双路径决策

`children_can_direct_render` 的判断条件（全部满足才为 `true`）：

1. 子节点没有需要叠加在它们之上的后处理特效（如 blur 叠加在 children 上）。
2. 父节点的变换矩阵仅包含 **Translation + Uniform Scale**（即无旋转、无倾斜、无透视）。
3. 子节点的渲染区域完全落在父节点的 Axis-Aligned Bounding Box 内。

**安全回退**: 任一条件不满足时，自动使用 `acquire_children_texture` 的离屏路径，保证渲染正确性。

---

## 第四章：错误处理策略

### 4.1 Pool 未初始化

若 `VelloBackend` 中的 `SharedTexturePool` 为 `None`：
- **Blur 节点**: 跳过该节点（使用 `log::warn_once!` 避免 IO 开销）。
- **Children Texture**: 强制走直接渲染路径；若必须离屏则跳过特效，直接渲染子节点（视觉降级但可见）。

### 4.2 零尺寸纹理请求

若 blur 节点 bounding box 取整后为 `0×0`：
- 跳过该 blur entry，避免 wgpu validation error。
- 在 `BlurredTextureEntry` 中标记 `skipped_due_to_size`，Pass 4 合成时明确将其视为空白区域。

### 4.3 变换矩阵不兼容

当 `children_can_direct_render` 检测到非轴对齐变换（旋转、倾斜、透视）时，自动回退到离屏路径。对用户完全无感知。

### 4.4 调试断言

- `FilterPipeline` 内部使用 `debug_assert!` 验证输入纹理尺寸。
- `TexturePool::acquire` 使用 `debug_assert!` 验证归还纹理与 bucket key 严格匹配。
- Release 构建中不匹配仅导致视觉瑕疵，不会崩溃。

---

## 第五章：测试策略

### 5.1 单元测试

- **资源归还**: 验证 `N` 次 `acquire` 后经过 `Drop` 循环，`pool.current_bytes()` 不变。
- **Bucket 隔离**: 验证不同尺寸的纹理不会落入同一 bucket。
- **Kawase 尺寸**: 验证 `acquire_kawase_set` 返回的 4 张纹理尺寸符合 `/2` 和 `/8` 规则。
- **View 缓存**: 验证连续两次调用 `PooledTexture::view()` 返回同一对象。

### 5.2 集成测试

- **Kawase 像素稳定性**: 跑完整流程（`Rgba8Unorm` → `Rgba8Unorm`），验证 Pass 合并后像素值无漂移。
- **极端半径**: 测试 `blur_radius = 5px, 20px, 100px` 下的平滑性。
- **矩阵检测**: `Translate(10, 20)` → `true`；`Scale(1.0)` → `true`；`Rotate(0.0001)` → `false`。

### 5.3 帧稳定性测试

- **Android Logcat**: 对比修改前后 `SurfaceFlinger --latency` 的 P50/P90/P99。
- **内存曲线**: Android Studio Profiler 抓取 30 秒 GPU Memory，验证曲线为水平线（无锯齿状分配释放）。
- **Gralloc 日志检查**: 确认 Logcat 中不再出现纹理分配相关日志。

### 5.4 视觉回归测试

- 对比修改前后同一份 blur demo 的截图：
  - 模糊半径一致
  - 边缘无黑边/白边
  - Children 直接渲染与离屏路径在轴对齐场景下像素输出一致

### 5.5 压力测试（补充）

- **多 Entry 并发**: 在 1000 个方块全部开启模糊的极端场景下，验证同一帧内多个 Entry 竞争同一 bucket 时池子能正确扩展，不会死锁或返回正在使用的纹理。
- **溢出策略**: 定义当内存超过预设阈值（如 256MB）时的行为（拒绝分配或强制清理）。

---

## 附录：文件改动清单

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `crates/dyxel-render-vello/src/texture_pool.rs` | 修改 | 新增 `acquire_blur_offscreen`、`acquire_children_texture`；`PooledTexture` 预缓存 `TextureView` |
| `crates/dyxel-render-vello/src/filter_pipeline.rs` | 修改 | 删除 `KawaseTexturePool`；依赖外部 `SharedTexturePool` |
| `crates/dyxel-render-vello/src/lib.rs` | 修改 | `BlurredTextureEntry.texture` 改为 `PooledTexture`；接入池化；实现 `blur_cache` 内容哈希；添加 `children_can_direct_render` |
| `crates/dyxel-render-vello/src/offscreen_renderer.rs` | 删除 | 完全未使用的死代码 |
