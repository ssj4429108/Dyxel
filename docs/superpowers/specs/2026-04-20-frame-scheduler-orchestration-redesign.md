# FrameScheduler 编排重构设计

## 1. 背景

当前渲染架构的核心纯化已经完成：

- `Runtime` 负责布局、文本准备和 `RenderPackage` 产出
- `dyxel-render-vello` 基本退化为纯 render backend
- profiling 显示稳态下 `State` 和 `Scene` 的 CPU 成本已接近噪声级

现阶段的主要问题已经从“渲染太慢”转移为“帧编排不稳”。

当前系统同时存在两类节拍控制：

- Logic 侧通过 `bridge.rs` 中的等待与 render 完成边界参与节拍
- Render 侧通过 `FramePacer` 自行 pacing

这形成了双节拍源。实际表现是：渲染本身常常只需几毫秒，但 `FrameInterval`、`PacerWait`、`LogicTime` 仍然不稳定，许多 jank 实际来自调度错拍，而不是渲染超预算。

此外，现有模型也不适合作为后续 UI 控件和动画系统的长期基建：

- 没有单一 frame owner
- 没有统一的 60/90/120Hz cadence 语义
- 内容生产和帧触发语义混杂
- 诊断难以明确归因

因此系统需要从“局部 pacing 修补”转向“显式 frame orchestration”。

## 2. 目标

本次重构目标如下：

1. 引入显式 `FrameScheduler`，作为系统唯一 frame owner
2. 移除双节拍源，统一由 scheduler 驱动 frame lifecycle
3. 采用 latest-wins mailbox 模型
4. cadence 锁定真实显示刷新率，并支持整数分频降档
5. 让 `Input`、`Logic`、`Render` 统一围绕 scheduler 协作
6. 为后续 UI 控件、动画和交互系统提供稳定 frame contract
7. 提供可解释、可归因的 frame diagnostics

## 3. 非目标

本设计明确不包含以下内容：

1. 不回放历史逻辑提交  
   两次可呈现拍之间若发生多次状态变化，只要求显示最新状态，不要求逐一显示中间状态。

2. 不让 `FrameScheduler` 变成第二个 runtime  
   scheduler 不负责布局、文本测量、命中测试或控件语义。

3. 不把 GPU 策略搬进 scheduler  
   texture pool、bind group、raster cache eviction 等仍属于 runtime 或 backend 自身职责。

4. 不以最低输入延迟为第一目标  
   本设计优先保证 cadence 稳定和相位对齐。

5. 不把输入或逻辑提交视为直接 render 命令  
   它们只是 scheduler 的输入事件，不直接触发出帧。

## 4. 总体架构

新架构由四类角色组成：

- `Input Source`
- `FrameScheduler`
- `Logic Worker`
- `Render Worker`

### Input Source

负责收集平台输入、生命周期事件和刷新信号，只向 scheduler 投递事件，不直接驱动 logic 或 render。

### FrameScheduler

独立线程运行，拥有系统唯一的 frame ownership。它负责：

- 接收输入、逻辑提交、vblank、render completion、surface 变化等事件
- 决定本拍是否允许出帧
- 决定消费哪个 epoch
- 决定是否跳拍
- 决定当前 cadence divisor

### Logic Worker

负责处理输入、更新 runtime state、构建最新 `RenderPackage`、提交到 mailbox，并发出 `LogicCommitted(epoch)`。

它不负责决定何时出帧。

### Render Worker

负责等待 `FrameToken`，读取 mailbox 快照，执行 `backend.render_package(...)`，并回报完成与统计信息。

它不负责自行 pacing。

## 5. 设计原则

### 5.1 单一 Frame Owner

`FrameScheduler` 是系统唯一 frame owner，Logic 和 Render 都不能自行启动新 frame。

### 5.2 基于 VSync 的 Frame Start

新 frame 只能在 cadence 允许的 vblank 边界上开始。

### 5.3 请求合并

同一拍前的重复 frame request 必须合并，不形成 render backlog。

### 5.4 Latest-Wins Mailbox

render 始终只消费最新可显示状态，不按提交顺序回放旧 package。

### 5.5 不补历史帧

missed cadence 后不补画旧帧，下一拍直接显示最新状态。

### 5.6 职责清晰

- Logic 只生产内容
- Render 只消费内容
- Scheduler 只控制时间

### 5.7 刷新率对齐

系统必须锁定真实显示刷新率，并只通过整数分频降档。

### 5.8 可观测性优先

每一帧都必须能从 vblank 一路追踪到 present。

## 6. 核心组件

### 6.1 FrameScheduler

拥有：

- scheduler 状态机
- cadence governor
- in-flight frame 跟踪
- 当前 surface cadence 配置
- presented epoch 记账
- frame timeline

建议结构：

```rust
struct FrameScheduler {
    state: SchedulerState,
    cadence: CadenceGovernor,
    mailbox: RenderMailbox,
    timeline: FrameTimeline,
    surface: SurfaceState,
    in_flight: Option<InFlightFrame>,
    last_presented_epoch: u64,
    latest_committed_epoch: u64,
}
```

### 6.2 RenderMailbox

保存最新提交的 `RenderPackage`，外部语义为 single-slot latest-wins。

建议 V1 结构：

```rust
struct RenderMailbox {
    latest_epoch: AtomicU64,
    latest_package: RwLock<Arc<RenderPackage>>,
}
```

### 6.3 CadenceGovernor

负责：

- 维护 `display_hz`
- 维护 `divisor`
- 基于统计窗口决定当前 divisor
- 每个 vblank 判断当前拍是否允许 present

### 6.4 FrameTimeline

记录每个 frame token 的完整生命周期，供 diagnostics 和 governor 使用。

## 7. 事件模型

所有调度输入统一为显式事件：

```rust
enum SchedulerEvent {
    InputArrived(InputBatchId),
    LogicCommitted { epoch: u64 },
    VBlank { timestamp: Instant, refresh_hz: f64 },
    RenderStarted { frame_id: u64, epoch: u64 },
    RenderCompleted { frame_id: u64, epoch: u64, stats: FrameStats },
    SurfaceChanged { width: u32, height: u32, refresh_hz: f64 },
    Shutdown,
}
```

事件语义：

- `InputArrived`：有新输入，不等于立刻渲染
- `LogicCommitted`：有新 package，不等于立刻渲染
- `VBlank`：唯一自然 frame start 边界
- `RenderCompleted`：只更新状态和统计，不再驱动 logic 节拍

## 8. 状态机

`scheduler` 使用五个状态：

```rust
enum SchedulerState {
    Idle,
    WaitingForLogic,
    Armed,
    Rendering,
    CoolingDown,
}
```

### Idle

当前没有待显示新内容。

### WaitingForLogic

输入已到达，logic 正在产出新 package，但尚未 commit。

### Armed

已有新内容待显示，等待下一个允许的 cadence tick。

### Rendering

当前存在 in-flight frame，render worker 正在执行。

### CoolingDown

一帧刚完成，等待下一次 cadence 决策。

### 状态机不变量

1. 任意时刻最多只有一个 in-flight frame
2. frame 只能在 cadence 允许的 vblank 上开始
3. presented epoch 必须单调递增
4. 新 commit 可以覆盖旧 armed epoch
5. render completion 不得驱动 logic cadence

## 9. Mailbox 语义

`RenderMailbox` 必须满足：

1. 逻辑上是单槽结构
2. commit 是替换，不是追加
3. render 读取稳定快照，而不是拿到邮箱所有权
4. 更高 epoch 可在发 token 前覆盖旧 armed epoch
5. 正在渲染的 frame 不被中途抢占
6. 屏幕真相由 `last_presented_epoch` 定义
7. backlog 用 epoch 距离表示，而不是队列长度
8. dropped epochs 是 latest-wins 的正常结果，不视为故障

## 10. 同步模型

不同职责使用不同同步原语，不使用单一原语覆盖所有场景。

### 10.1 事件与命令

使用 channel 传递：

- `SchedulerEvent`
- `LogicCommand`
- `RenderCommand`

推荐使用 `crossbeam_channel` 或等价方案。

### 10.2 Mailbox 数据

V1 使用：

- `RwLock<Arc<RenderPackage>>`
- `AtomicU64 latest_epoch`

优先保证语义正确与实现清晰，后续如有必要再升级到无锁 mailbox。

### 10.3 热路径状态

使用 `AtomicU64` / `AtomicBool` 存储：

- `latest_committed_epoch`
- `last_presented_epoch`
- `render_in_flight`
- `shutdown`

## 11. Worker 契约

### 11.1 Logic Worker

负责：

- 处理输入与 runtime 工作
- 更新状态
- 执行 `runtime_prepare()`
- 生成 `RenderPackage`
- commit 到 mailbox
- 发出 `LogicCommitted(epoch)`

不负责：

- 直接发起 draw
- 等待 render completion
- 拥有 cadence 决策权

### 11.2 Render Worker

负责：

- 等待 `RenderCommand::Render(FrameToken)`
- 从 mailbox 读取稳定快照
- 执行 `backend.render_package(...)`
- 发出 `RenderStarted`
- 发出 `RenderCompleted(stats)`

不负责：

- 自行 pacing
- 持有 target FPS
- 消费历史 package 队列

### 11.3 Input Contract

输入源只负责产生 scheduler 事件，不直接触发 render。

## 12. Cadence 模型

系统不固定写死 60fps，而是锁定真实显示刷新率。

### 12.1 Effective Cadence

```rust
effective_hz = display_hz / divisor
target_frame_duration = 1.0 / effective_hz
```

### 12.2 支持的 Divisor

- 60Hz：`[1, 2]` -> `60, 30`
- 90Hz：`[1, 2, 3]` -> `90, 45, 30`
- 120Hz：`[1, 2, 3, 4]` -> `120, 60, 40, 30`

不支持非整数分频。

### 12.3 VBlank 决策

每个 vblank 都会被观察，但不是每个 vblank 都允许 present。

建议规则：

```rust
should_present_this_tick =
    (vblank_counter - 1) % divisor as u64 == 0;
```

这样可保持 refresh-locked 相位稳定。

## 13. CadenceGovernor

### 13.1 输入

Governor 维护：

- 当前 `display_hz`
- 当前 `divisor`
- 近帧统计窗口
- missed cadence 率
- render overlap 情况

### 13.2 统计窗口

建议维护两个窗口：

- 短窗口：`N = 8`
- 长窗口：`N = 120`

建议统计：

- `short_missed_rate`
- `short_p95_total_ms`
- `short_p95_gpu_ms`
- `long_missed_rate`
- `long_p95_total_ms`
- `long_p95_gpu_ms`

### 13.3 预算定义

```rust
target_frame_ms = 1000.0 / effective_hz
downgrade_pressure = 0.85 * target_frame_ms
upgrade_headroom = 0.60 * target_frame_ms
```

### 13.4 降档规则

满足以下任一条件时，降一级 divisor：

1. `short_missed_rate >= 0.25`
2. `short_p95_total_ms >= downgrade_pressure`
3. `short_p95_gpu_ms >= 0.75 * target_frame_ms`
4. 连续 `SKIPPED_IN_FLIGHT >= 3`

只降一级，不跨级跳档。

### 13.5 升档规则

仅当以下条件全部满足时，升一级 divisor：

1. 已过最小驻留时间
2. `long_missed_rate <= 0.01`
3. `long_p95_total_ms <= upgrade_headroom`
4. `long_p95_gpu_ms <= 0.50 * target_frame_ms`
5. 长窗口内无明显 `SKIPPED_IN_FLIGHT`

升档必须比降档更保守。

### 13.6 驻留时间

建议默认值：

- `min_residency = 2s`

严重过载时允许跳过驻留时间继续降档，但绝不允许跳过驻留时间升档。

## 14. CadenceInfo 与 Logic 集成

scheduler 需要把 cadence 信息回传给 logic worker，以保证动画与 frame 生产逻辑理解当前真实 cadence。

建议结构：

```rust
struct CadenceInfo {
    display_hz: f64,
    divisor: u32,
    effective_hz: f64,
    target_frame_duration: Duration,
    expected_present_time: Instant,
}
```

Logic 可接收：

```rust
struct LogicFrameHint {
    cadence: CadenceInfo,
    frame_deadline: Instant,
}
```

这样 logic 不会在 scheduler 降到 60/40/30fps 时仍按固定 60fps 语义推进动画。

### 可选 Lead Time

可选支持 `logic_lead_time`，用于让 logic 在预期可呈现拍之前提前准备 package。

建议默认值：

- 60Hz：`2ms`
- 90Hz：`3ms`
- 120Hz：`4ms`

该能力可作为后续增强，不要求 V1 必须启用。

## 15. 帧生命周期

标准帧流程如下：

1. 输入到达
2. scheduler 派发 logic 工作
3. logic 更新状态并提交最新 `RenderPackage`
4. logic 发出 `LogicCommitted(epoch)`
5. scheduler 进入 `Armed`
6. 允许的 `VBlank` 到来
7. scheduler 检查：
   - 当前拍是否允许 present
   - 当前是否已有 in-flight frame
   - 是否存在新内容
8. scheduler 发出 `FrameToken`
9. render worker 执行并发出 `RenderStarted`
10. render 完成并发出 `RenderCompleted`
11. scheduler 更新：
   - `last_presented_epoch`
   - timeline
   - governor
   - 下一状态

该流程确保“内容准备”与“何时出帧”彻底解耦。

## 16. 诊断与指标

### 16.1 每帧字段

建议记录：

- `FrameId`
- `Epoch`
- `DisplayHz`
- `Divisor`
- `EffectiveHz`
- `VBlankAt`
- `TokenIssuedAt`
- `RenderStartedAt`
- `RenderCompletedAt`
- `PresentedAt`
- `DroppedEpochsSinceLastPresent`
- `FrameResult`

### 16.2 FrameResult 分类

建议使用：

- `ON_TIME`
- `MISSED_CADENCE`
- `SKIPPED_IDLE`
- `SKIPPED_DIVISOR`
- `SKIPPED_IN_FLIGHT`

这样可避免把正常 cadence wait 误判成 jank。

### 16.3 聚合指标

重点关注：

- `MissedCadenceRate`
- `DroppedEpochRate`
- `CadenceSwitchCount`
- `TimeInDivisor[x]`
- `p95 RenderCPU`
- `p95 GPU`

### 16.4 Trace 导出

`FrameTimeline` 应支持导出到 Chrome Trace / Perfetto 兼容格式，用于复杂异步调度下的真实验证。

## 17. 迁移计划

### Phase 1：引入类型与观测

新增：

- `FrameScheduler`
- `RenderMailbox`
- `FrameToken`
- `SchedulerEvent`
- `LogicCommand`
- `RenderCommand`
- `FrameTimeline`

先做接线和观测，不立刻切主控制流。

### Phase 2：收归 Frame Ownership

- 启动 scheduler thread
- 让 vblank、logic commit 和 render completion 统一进入 scheduler
- logic 不再等待 render completion 决定节拍
- render worker 开始支持消费 `FrameToken`

### Phase 3：移除旧 draw 触发路径

- `RequestDraw` 退出主路径
- render completion 不再驱动 logic cadence
- mailbox + scheduler 成为唯一权威输入

### Phase 4：启用自适应 Cadence

- pacing 逻辑从 render 侧移入 scheduler
- 启用 `CadenceGovernor`
- 完成高刷与 diagnostics 重构

新旧 frame ownership 模型不能长期并存，部分 adoption 是危险的。

## 18. WASM 共享内存兼容性

本次重构不应破坏现有 `wasm <-> native` 共享内存设计。当前系统中，共享内存承担的是 guest/host 边界上的数据交换职责，而不是 frame orchestration 职责，这一边界在新架构下必须保持不变。

### 18.1 共享内存的角色

`SharedBuffer` 与 dual-track 内存布局继续作为 WASM 与 Host 之间的数据面，负责：

- WASM -> Native 的命令流传输
- Native -> WASM 的布局结果回写
- 输入 ring buffer 交换
- device/runtime 元数据交换

它们不负责：

- frame token 分发
- cadence 决策
- render worker 调度
- governor 状态维护

这些都属于 host 内部控制面，必须由 `FrameScheduler`、Logic worker 和 Render worker 自行管理。

### 18.2 Logic Worker 是唯一的 Guest/Host 内存桥

在新架构中，只有 Logic worker 允许直接访问 WASM linear memory 或 `SharedBuffer` 原始指针。它继续负责：

- 调用 guest tick
- 读取并处理 command stream
- 执行 `process_commands(...)`
- 执行 `sync_layout_to_wasm(...)`

`FrameScheduler` 不得直接读写 WASM memory。`Render Worker` 也不得直接访问 WASM memory。

### 18.3 RenderPackage 必须保持 Host 侧快照语义

`RenderMailbox` 中保存的必须是 Host 侧稳定快照，而不是指向 WASM linear memory 的活引用。正确的数据路径应保持为：

`WASM -> commands/shared memory -> SharedState -> RenderPackage -> RenderMailbox -> Vello`

不允许出现以下反向耦合：

`WASM memory -> RenderMailbox -> Render Worker`

否则会让 scheduler/render 的异步生命周期与 guest 内存生命周期纠缠在一起，破坏当前已经建立的 backend 纯化边界。

### 18.4 Layout 回写时机必须早于 RenderPackage commit

为了保证 guest 侧看到的 layout 结果与 host 侧最新状态一致，Logic worker 必须保持以下顺序：

1. 处理 guest command stream
2. 更新 host `SharedState`
3. 计算并回写 layout 到 WASM shared memory
4. 生成最新 `RenderPackage`
5. commit 到 `RenderMailbox`
6. 发出 `LogicCommitted(epoch)`

不能把 `sync_layout_to_wasm(...)` 延后到 render 完成之后，否则 guest 侧将看到滞后的 layout epoch。

### 18.5 设计约束

实现中必须满足以下约束：

1. `FrameScheduler` 只能调度 host-side workers，不能成为共享内存的新写入者。
2. `LogicCommitted(epoch)` 只表示 host 侧 package 已准备好，不等于 guest 侧 layout 已可见；guest 可见性仍以 layout 回写完成为准。
3. `Render Worker` 只能消费 `RenderPackage` 快照，不能回退到依赖 `SharedState` 或 `SharedBuffer` 的活数据。
4. dual-track / `SharedBuffer` 的数据格式可保持独立演进，但不得承载 scheduler 控制语义。

### 18.6 兼容性结论

只要遵守上述边界，本次 `FrameScheduler` 重构就不会破坏现有共享内存设计。相反，它会让三层职责更清晰：

- `SharedBuffer / dual-track`：WASM 与 Host 之间的数据面
- `FrameScheduler`：Host 内部的控制面
- `RenderMailbox`：Host 内部的最新渲染快照面

这三层应保持严格分离。

## 19. 风险

1. 调度层复杂度会明显提升，时序 bug 更隐蔽
2. latest-wins 语义会主动丢弃中间 epoch，需要团队接受
3. governor 阈值若设置不当，可能导致高刷下 divisor 振荡
4. 旧 diagnostics 在迁移期会失真，必须同步替换
5. 半做不做最危险，会重新引入多处 frame ownership

## 20. 验证策略

### 20.1 正确性验证

检查：

- 任意时刻最多只有一个 in-flight frame
- presented epoch 单调递增
- stale epoch 不会被 replay
- render completion 不驱动 logic cadence

### 20.2 Cadence 稳定性验证

在 60Hz 与 120Hz 设备上分别验证：

- 60Hz 下 `FrameInterval p95` 接近 `16.67ms`
- 120Hz 轻场景优先跑 `8.33ms`
- 重场景稳定退到 `16.67ms` 或其他 divisor，而不是频繁振荡

### 20.3 行为验证

覆盖：

- 连续动画
- 高频输入拖拽
- blur-heavy scene
- resize / surface recreation
- suspend / resume
- idle scene

## 21. 成功标准

当满足以下条件时，本重构视为完成：

1. logic 不再等待 render completion 决定自身节拍
2. render 不再独立 self-pace
3. 重复 frame request 被合并为 latest-wins 语义
4. missed cadence 后不补历史帧
5. 60/90/120Hz 设备均采用 refresh-locked cadence，并通过整数分频稳定回退
6. diagnostics 能明确区分 logic、render 和 scheduling 导致的问题
