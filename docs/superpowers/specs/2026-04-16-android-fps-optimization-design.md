# Android FPS 优化设计文档

**日期:** 2026-04-16  
**主题:** Android 端 FPS 抖动与掉帧问题根因修复  
**状态:** 已批准

---

## 1. 问题总览

当前 `com.dyxel.android` 在 Android 设备上存在 FPS 不稳定、偶发性掉帧、以及被锁在 30fps 附近的问题。经过代码审计，识别出 6 个根因，分为 3 个优先级阶段修复。

---

## 2. 根因分析

### Problem 1: Logic Thread 被 Render Thread 阻塞
- **位置:** `crates/dyxel-core/src/bridge.rs:728-752`
- **现象:** Logic Thread 每次 tick 后 `recv_timeout(33ms)` 等待 Render Thread 完成，强制将 WASM 逻辑帧率绑定到渲染帧率。
- **影响:** 一旦 GPU 负载波动，Logic Thread 跟着 stall，帧率直接被锁在 30fps。

### Problem 2: Android VSync 未接入
- **位置:** `crates/dyxel-core/src/android_vblank.rs` + `android/app/src/main/java/com/dyxel/android/MainActivity.kt`
- **现象:** Rust 侧 `AndroidVBlankWaiter` 和 JNI 回调 `nativeOnVBlank` 已完备，但 Kotlin 侧未注册 `Choreographer.FrameCallback`。
- **影响:** FramePacer 退化为纯软件 spin-sleep，Mailbox 低延迟优势无法发挥，产生 ms 级 jitter。

### Problem 3: GPU 内存频繁分配（未池化的 full-res 纹理）
- **位置:** `crates/dyxel-render-vello/src/filter_pipeline.rs:1130-1145`
- **现象:** Kawase blur Pass 5 每帧每 entry 新建全分辨率 `Rgba16Float` 纹理，未入池。
- **影响:** 每帧数十 MB 的 GPU 内存 churn，驱动层 stall。

### Problem 4: Per-entry Uniform Buffer 堆分配
- **位置:** `crates/dyxel-render-vello/src/lib.rs:1825-1858`
- **现象:** 每帧每个 blur entry 创建两个 `wgpu::Buffer`。
- **影响:** CPU 侧高频堆分配和命令编码器开销。

### Problem 5: Blur Render Pass 过多
- **位置:** `crates/dyxel-render-vello/src/filter_pipeline.rs`
- **现象:** 每个 blur entry 6 个 render pass，全部塞在一个 encoder 里。
- **影响:** GPU（尤其是 Mali/Adreno）对 render pass 切换敏感。

### Problem 6: TexturePool quarter size 不匹配
- **位置:** `filter_pipeline.rs:69-70` vs `texture_pool.rs:206-207`
- **现象:** 内部池用 `/8`，外部池用 `/4`。
- **影响:** 不同路径纹理分辨率不一致，可能导致采样质量差异。

---

## 3. 实施方案

### 第一阶段：打通 VSync 与逻辑解耦（Problem 1 & 2）

#### 3.1 bridge.rs 异步化改造
- **移除** `render_complete_rx.recv_timeout`。
- **引入** `Arc<AtomicBool>` 标记 `is_rendering`。
- **逻辑:**
  - Logic Thread 发送 `RequestDraw` 前检查 `is_rendering`。
  - 如果为 `true`，跳过该帧的 `RequestDraw` 发送（跳帧）。
  - Logic Thread 的 WASM tick 继续执行，不再被 Render Thread 阻塞。
- **新增 DIAG 字段:** `LogicTime`（WASM 运行耗时）、`RenderJank`（Logic 想渲染但 Render 忙的次数）。

#### 3.2 Kotlin 端接入 Choreographer
在 `MainActivity.kt` 中注册 `Choreographer.FrameCallback`：

```kotlin
private fun startChoreographer() {
    Choreographer.getInstance().postFrameCallback(object : Choreographer.FrameCallback {
        override fun doFrame(frameTimeNanos: Long) {
            DyxelJNI.nativeOnVBlank(ptr)
            Choreographer.getInstance().postFrameCallback(this)
        }
    })
}
```

收益：FramePacer 的 `wait_for_vblank()` 真正被硬件 VBlank 信号唤醒。

### 第二阶段：清理 GPU 资源 churn（Problem 3 & 4）

#### 3.3 TexturePool 扩展 Rgba16Float Full-res 支持
- 在 `TexturePool` / `SharedTexturePool` 中增加 full-res `Rgba16Float` 纹理的 acquire/return。
- **分桶策略:** `(width, height, usage)`，因为该纹理有时作为 `Storage` 写入，有时作为 `Attachment` 写入。
- 替换 `filter_pipeline.rs` 中每帧 `device.create_texture` 的调用为池化获取。

#### 3.4 Uniform Buffer 零拷贝
- 在 `VelloBackend` 初始化一个 1MB 的 `wgpu::Buffer` 作为 `GlobalStagingBuffer`。
- 每帧 blur uniform 数据通过 `queue.write_buffer` 按 offset 动态写入，不再 per-entry `create_buffer`。

### 第三阶段：修复逻辑一致性（Problem 5 & 6）

#### 3.5 统一 quarter size 计算
- 将 `filter_pipeline.rs:69-70` 的 `quarter_w = (full_width / 8)` 改为 `(full_width / 4)`，与 `texture_pool.rs` 对齐。
- 验证 `KawaseTexturePool::matches()` 增加 quarter size 一致性检查。

#### 3.6 Blur Render Pass 合并（可选，视第二阶段效果决定是否执行）
- 评估多 entry blur 合并为 atlas/subpass 的可行性，留作后续优化项。

---

## 4. 成功标准

1. Android 端平均 FPS 稳定在 58-60fps。
2. Janky frames 占比从 28.6% 降至 10% 以下。
3. `LogicTime` DIAG 日志显示 WASM tick 不再受渲染阻塞。
4. GPU 内存分配 churn 消失（通过 `dumpsys meminfo` 或 wgpu 日志验证）。

---

## 5. 文件清单

| 阶段 | 修改文件 |
|------|----------|
| 1 | `crates/dyxel-core/src/bridge.rs` |
| 1 | `android/app/src/main/java/com/dyxel/android/MainActivity.kt` |
| 1 | `crates/dyxel-core/src/pacer.rs`（适配验证） |
| 2 | `crates/dyxel-render-vello/src/texture_pool.rs` |
| 2 | `crates/dyxel-render-vello/src/filter_pipeline.rs` |
| 2 | `crates/dyxel-render-vello/src/lib.rs` |
| 3 | `crates/dyxel-render-vello/src/filter_pipeline.rs` |
| 3 | `crates/dyxel-render-vello/src/texture_pool.rs` |
