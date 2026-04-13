# Dyxel 生产级帧调度 (Frame Pacing) 设计方案

**日期**: 2026-04-13  
**目标**: 消除 macOS (M5 / 60Hz) 上的 `TexWait` 剧烈抖动（4ms–14ms），在 `PresentMode::Immediate` 模式下实现稳定、低延迟的帧输出。

---

## 1. 问题诊断

当前 Dyxel 在 M5 芯片上渲染仅需约 2.5ms（含 Blur），属于"算力过剩"。但由于渲染线程没有主动 pacing，它在 `Immediate` 模式下会疯狂轮转，导致 `get_current_texture()` 的等待时间（`TexWait`）剧烈波动。用户看到的不是"更快"，而是微小的撕裂感和不连贯。

**核心矛盾**: CPU/GPU 太快，而 60Hz 屏幕的物理刷新周期是固定的 16.67ms。没有 pacing = 没有优雅。

---

## 2. 设计原则

1. **不解锁帧率上限**：保留 `PresentMode::Immediate`，未来可一键切到 Benchmark 模式跑 300+ FPS。
2. **精准心跳**：用自研 `FramePacer` 把每一帧的启动时间钉死在目标 VBlank 网格上。
3. **单次提交**：把一帧内所有 GPU 工作收敛到一个 `CommandEncoder` + 一次 `queue.submit()`，减少驱动层隐式同步开销。
4. **可观测性**：DIAG 日志必须一眼看出 `PacerWait`（主动让出时间）和 `FrameInterval`（真实帧间隔稳定性）。

---

## 3. 整体架构

```
Render Thread Loop
    │
    ▼
FramePacer::wait_for_next_frame()
    │    ├── Spin-Sleep 策略：>2ms 时 sleep，最后 0.5ms 自旋
    │    └── 返回 PacerWait 时长
    ▼
get_current_texture()              ← 获取 surface（如有少量 TexWait，压在此处）
    ▼
Device::create_command_encoder()
    ▼
├── Vello Fine Raster (render_to_texture)
├── Blur / Shadow Compute Passes (共享 encoder)
└── Final Blit RenderPass (画入 surface)
    ▼
queue.submit(Some(encoder.finish()))
    ▼
surface_texture.present()
    ▼
FramePacer::mark_present()
    ▼
DIAG 日志输出
```

---

## 4. FramePacer 详细设计

### 4.1 文件位置
`crates/dyxel-core/src/pacer.rs`

### 4.2 核心结构
```rust
pub struct FramePacer {
    /// 固定的理想 VBlank 死线，不跟随 last_present 漂移
    target_deadline: Instant,
    target_frame_duration: Duration,
    buffer_time: Duration, // 默认 0.5ms
}
```

### 4.3 算法：Adaptive Spin-Sleep
```rust
pub fn wait_for_next_frame(&mut self) -> Duration {
    let wait_start = Instant::now();
    let target = self.target_deadline - self.buffer_time;

    if wait_start < target {
        let remaining = target - wait_start;
        // >2ms：交给系统 sleep
        if remaining > Duration::from_millis(2) {
            thread::sleep(remaining - Duration::from_micros(500));
        }
        // 最后 0.5ms：自旋锁死，防止系统调度放鸽子
        while Instant::now() < target {
            std::hint::spin_loop();
        }
    }

    let pacer_wait = Instant::now().duration_since(wait_start);
    // 推进下一帧的理想死线
    self.target_deadline += self.target_frame_duration;
    pacer_wait
}

pub fn mark_present(&mut self) {
    // 主要用于外部诊断，不漂移死线
}
```

### 4.4 关键修正：理想 VBlank 时间线
如果某一帧因为不可抗力迟到（`elapsed > target`），`wait_for_next_frame` 会**立即返回**，引擎满速追赶。**下一帧的死线不顺延**，而是在现有 `target_deadline` 基础上直接再推进 `target_frame_duration`。这避免了错误累积。

---

## 5. 单次提交 (Single Submission) 重构

### 5.1 目标文件
`crates/dyxel-render-vello/src/lib.rs`

### 5.2 当前问题
分散在渲染流程中的多次 `queue.submit()` 会切断 GPU 指令流水，引入不必要的内核态切换和隐式同步。

### 5.3 重构后管线
按严格顺序执行：

1. **Pacer 唤醒**
2. **获取 Surface Texture** (`get_current_texture()`)
3. **创建单一 CommandEncoder**
4. **Vello Fine Raster** → `renderer.render_to_texture(encoder, ...)` 写入 `triple_buffer.write_buffer()`
5. **Effect Passes** → 同一 `encoder` 中启动 `ComputePass`（Blur 读取 + 写入临时纹理）
6. **Final Blit** → 同一 `encoder` 中启动 `RenderPass`，将结果 blit 到 `surface_texture`
7. **`queue.submit(Some(encoder.finish()))`**
8. **`surface_texture.present()`**

### 5.4 纹理 Usage 与屏障

| 阶段 | 纹理 | wgpu Usage |
|------|------|------------|
| Vello 输出 | offscreen (write) | `STORAGE_BINDING \| TEXTURE_BINDING` |
| Blur 读取 | offscreen (read) | `TEXTURE_BINDING` |
| Blur 写入 | blur temp | `STORAGE_BINDING \| TEXTURE_BINDING` |
| Blit 读取 | offscreen / blur result | `TEXTURE_BINDING` |
| Blit 写入 | surface | `RENDER_ATTACHMENT` |

wgpu 在同一 `CommandEncoder` 内会根据 `Usage` 变化自动插入 pipeline barrier。唯一要求是：**前面的 Pass 必须在开启下一个 Pass 前完成（drop/scope end）**。

### 5.5 前置条件
`dyxel-render-vello/src/lib.rs` 的 `render_internal()` 方法中，必须把零散的 `queue.submit()` 调用全部合并。`TripleBuffer` 的 `write_buffer()` 机制保持不变——它为 CPU/GPU 提供了完整一帧的隔离。

---

## 6. 刷新率探测

### 6.1 分层降级策略

| 层级 | 精度 | 实现 | 回退条件 |
|------|------|------|----------|
| **Primary** | 精确 | `winit::window::Window::current_monitor()` → `video_mode.refresh_rate_millihertz() / 1000.0` | 总是尝试 |
| **Secondary** | 微秒级 | `CVDisplayLink` (未来通过 FFI 或 `core-video-rs` ) | Primary 返回 None |
| **Fallback** | 兜底 | 硬编码 `60.0` | 全部失败 |

### 6.2 数据流
- `mac/src/main.rs` 在窗口创建后探测一次 `target_fps`。
- 通过 `host.set_target_fps(fps)` 注入到 `DyxelHost`。
- `DyxelHost` 随 `RenderMessage::SetReady` 把 fps 传给 Render Thread。
- Render Thread 初始化 `FramePacer::new(target_fps)`。

### 6.3 winit 数值参考
- M5 MacBook Air: `60000` mHz → **60 Hz**
- ProMotion MacBook Pro: `120000` mHz → **120 Hz**

---

## 7. DIAG 日志重构

### 7.1 输出格式
```
[DIAG] Frame 752: Total=16.67ms, PacerWait=14.10ms, State=0.05ms, Scene=0.30ms, GPU=1.20ms, BlurCopy=0.10ms, BlurRender=0.40ms, Pass3=0.20ms, GetTex=0.05ms, TexWait=0.10ms, Blit=0.20ms, Present=0.05ms, FrameInterval=16.67ms, FPS=60.0 [PERF: OK]
```

### 7.2 新增字段
- `PacerWait`：`wait_for_next_frame()` 实际耗时。高且稳定 = 引擎主动掌控节奏。
- `FrameInterval`：上一次 `present()` 到本次 `present()` 的时间差。这是用户**肉眼可见**的稳定性指标。理想值钉死在 `16.67ms`（60Hz），标准差应 `< 0.3ms`。

### 7.3 状态标识符
在日志末尾追加 `[PERF: X]`，基于当帧数据实时诊断：

| 标识 | 条件 | 含义 |
|------|------|------|
| `[PERF: OK]` | `PacerWait > 2ms` 且 `FrameInterval` 抖动 `< 0.5ms` | 健康，负载极低 |
| `[PERF: WARM]` | `PacerWait < 2ms` 且无掉帧 | 负载接近上限 |
| `[PERF: JANK]` | `FrameInterval > Target + 1ms` | 发生掉帧，需排查 WASM/GPU |

---

## 8. 预期效果

重构后，M5 MacBook Air (60Hz) 上的 DIAG 日志应呈现以下特征：

```
[DIAG] Frame 752: Total=16.67ms, PacerWait=14.1ms, Render=2.4ms, TexWait=0.1ms, FPS=60.0 [PERF: OK]
```

- `PacerWait` 占大头 → CPU 主动让出性能预算。
- `TexWait` 极小且稳定 → Swapchain 获取不再抖动。
- `FrameInterval` 标准差 `< 0.3ms` → 肉眼看不出任何跳帧。

这标志 Dyxel 从"大力出奇迹"的暴力引擎，转变为"按需索取"的精准引擎。

---

## 9. 实施范围

- **新增**: `crates/dyxel-core/src/pacer.rs`
- **修改**: `crates/dyxel-core/src/bridge.rs` (集成 Pacer 到 Render Thread)
- **修改**: `crates/dyxel-core/src/lib.rs` (导出 pacer 模块)
- **修改**: `crates/dyxel-render-vello/src/lib.rs` (单次提交重构)
- **修改**: `mac/src/main.rs` (刷新率探测并注入)
