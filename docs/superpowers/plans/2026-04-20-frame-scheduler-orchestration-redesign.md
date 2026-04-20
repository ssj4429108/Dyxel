# FrameScheduler 编排重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将当前基于 `RequestDraw + FramePacer + render completion wait` 的双节拍模型重构为独立 `FrameScheduler` 线程驱动的 scheduler-centric frame pipeline。

**Architecture:** 新增 `FrameScheduler`、`RenderMailbox`、`CadenceGovernor` 和 `FrameTimeline` 作为核心编排层；Logic worker 继续拥有 guest/host 共享内存桥职责，Render worker 只消费 `FrameToken` 和 `RenderPackage` 快照。迁移采用分阶段方式，先建立新控制面和观测，再逐步切断旧的 `RequestDraw` 与 logic-side wait 路径。

**Tech Stack:** Rust, crossbeam-channel, atomics, `RwLock<Arc<RenderPackage>>`, existing `dyxel-core` bridge/runtime/renderer, existing `dyxel-render-vello` backend

---

## File Map

### New files

- `crates/dyxel-core/src/frame_scheduler.rs`
  - `FrameScheduler` 主状态机
  - `SchedulerEvent`
  - `SchedulerState`
  - `FrameToken`
  - `LogicCommand`
  - `RenderCommand`
  - scheduler 主循环与事件处理

- `crates/dyxel-core/src/cadence.rs`
  - `CadenceGovernor`
  - `CadenceDecision`
  - `CadenceInfo`
  - divisor 规则、窗口统计、升降档逻辑

- `crates/dyxel-core/src/render_mailbox.rs`
  - `RenderMailbox`
  - latest-wins commit / snapshot

- `crates/dyxel-core/src/frame_timeline.rs`
  - `FrameTimeline`
  - `FrameRecord`
  - trace export helper

### Modified files

- `crates/dyxel-core/src/lib.rs`
  - 导出新模块

- `crates/dyxel-core/src/bridge.rs`
  - 删除 logic-side `wait_for_render_or_vsync()` 主路径
  - 删除/退役 `RequestDraw` 主语义
  - 启动 scheduler thread
  - 将 logic/render 线程改成 worker

- `crates/dyxel-core/src/renderer.rs`
  - 保持 `runtime_prepare()` 为 runtime-owned prepare
  - 接收 `CadenceInfo/LogicFrameHint`
  - 从“直接 render_frame”改成“在 render worker 中消费 `FrameToken`”

- `crates/dyxel-core/src/runtime.rs`
  - 保持 `process_commands()` / `sync_layout_to_wasm()`
  - 确保 layout 回写时序早于 mailbox commit

- `crates/dyxel-core/src/pacer.rs`
  - 从 render-thread pacer 改造成 scheduler-side vblank source/fallback helper

- `crates/dyxel-render-api/src/lib.rs`
  - 如需要，补充 frame stats/result 分类的 API 类型

- `crates/dyxel-render-vello/src/lib.rs`
  - 适配新的 frame timing / result 记录方式
  - 不改动 `RenderPackage` ownership 边界

### Test targets

- `crates/dyxel-core/src/frame_scheduler.rs`
- `crates/dyxel-core/src/cadence.rs`
- `crates/dyxel-core/src/render_mailbox.rs`
- `crates/dyxel-core/src/bridge.rs`

---

### Task 1: 搭建 Scheduler 基础类型与最新帧邮箱

**Files:**
- Create: `crates/dyxel-core/src/frame_scheduler.rs`
- Create: `crates/dyxel-core/src/render_mailbox.rs`
- Modify: `crates/dyxel-core/src/lib.rs`
- Test: `crates/dyxel-core/src/render_mailbox.rs`

- [ ] **Step 1: 写 mailbox 语义测试**

添加测试，覆盖 latest-wins 和快照稳定性：

```rust
#[test]
fn mailbox_commit_replaces_previous_epoch() {
    let mailbox = RenderMailbox::new();
    let p1 = Arc::new(RenderPackage::new((100, 100), None, Vec::new()));
    let p2 = Arc::new(RenderPackage::new((200, 200), None, Vec::new()));

    mailbox.commit(1, p1);
    mailbox.commit(2, p2.clone());

    let (epoch, snapshot) = mailbox.snapshot();
    assert_eq!(epoch, 2);
    assert_eq!(snapshot.viewport, (200, 200));
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p dyxel-core mailbox_commit_replaces_previous_epoch -- --nocapture`

Expected: FAIL，提示 `RenderMailbox` 未定义

- [ ] **Step 3: 实现 `RenderMailbox` 最小版本**

在 `crates/dyxel-core/src/render_mailbox.rs` 中添加：

```rust
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use dyxel_render_api::RenderPackage;

pub struct RenderMailbox {
    latest_epoch: AtomicU64,
    latest_package: RwLock<Arc<RenderPackage>>,
}

impl RenderMailbox {
    pub fn new() -> Self {
        Self {
            latest_epoch: AtomicU64::new(0),
            latest_package: RwLock::new(Arc::new(RenderPackage::new((0, 0), None, Vec::new()))),
        }
    }

    pub fn commit(&self, epoch: u64, package: Arc<RenderPackage>) {
        *self.latest_package.write().unwrap() = package;
        self.latest_epoch.store(epoch, Ordering::Release);
    }

    pub fn snapshot(&self) -> (u64, Arc<RenderPackage>) {
        let epoch = self.latest_epoch.load(Ordering::Acquire);
        let pkg = self.latest_package.read().unwrap().clone();
        (epoch, pkg)
    }
}
```

- [ ] **Step 4: 实现 scheduler 基础类型骨架**

在 `crates/dyxel-core/src/frame_scheduler.rs` 中添加最小骨架：

```rust
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerState {
    Idle,
    WaitingForLogic,
    Armed,
    Rendering,
    CoolingDown,
}

#[derive(Debug, Clone, Copy)]
pub struct FrameToken {
    pub frame_id: u64,
    pub epoch: u64,
    pub vblank_at: Instant,
    pub target_frame_duration: Duration,
}

#[derive(Debug)]
pub enum SchedulerEvent {
    LogicCommitted { epoch: u64 },
    Shutdown,
}

#[derive(Debug)]
pub enum LogicCommand {
    ProcessInput,
    Shutdown,
}

#[derive(Debug)]
pub enum RenderCommand {
    Render(FrameToken),
    Shutdown,
}
```

- [ ] **Step 5: 导出新模块**

在 `crates/dyxel-core/src/lib.rs` 中添加：

```rust
pub mod frame_scheduler;
pub mod render_mailbox;
```

- [ ] **Step 6: 运行测试，确认通过**

Run: `cargo test -p dyxel-core mailbox_commit_replaces_previous_epoch -- --nocapture`

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/dyxel-core/src/lib.rs crates/dyxel-core/src/frame_scheduler.rs crates/dyxel-core/src/render_mailbox.rs
git commit -m "feat: add frame scheduler core types and render mailbox"
```

### Task 2: 实现 CadenceGovernor 与高刷整数分频

**Files:**
- Create: `crates/dyxel-core/src/cadence.rs`
- Modify: `crates/dyxel-core/src/lib.rs`
- Test: `crates/dyxel-core/src/cadence.rs`

- [ ] **Step 1: 写 divisor 决策测试**

```rust
#[test]
fn cadence_governor_120hz_uses_integer_divisors() {
    let mut gov = CadenceGovernor::new(120.0);
    assert_eq!(gov.supported_divisors(), &[1, 2, 3, 4]);
}

#[test]
fn cadence_governor_only_presents_on_divisor_ticks() {
    let mut gov = CadenceGovernor::new(120.0);
    gov.set_divisor_for_test(2);

    let d1 = gov.on_vblank(std::time::Instant::now());
    let d2 = gov.on_vblank(std::time::Instant::now());

    assert!(d1.should_present_this_tick);
    assert!(!d2.should_present_this_tick);
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p dyxel-core cadence_governor_120hz_uses_integer_divisors -- --nocapture`

Expected: FAIL，提示 `CadenceGovernor` 未定义

- [ ] **Step 3: 实现 `CadenceGovernor` 最小版本**

在 `crates/dyxel-core/src/cadence.rs` 中添加：

```rust
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct CadenceDecision {
    pub should_present_this_tick: bool,
    pub divisor: u32,
    pub effective_hz: f64,
    pub target_frame_duration: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct CadenceInfo {
    pub display_hz: f64,
    pub divisor: u32,
    pub effective_hz: f64,
    pub target_frame_duration: Duration,
    pub expected_present_time: Instant,
}

pub struct CadenceGovernor {
    display_hz: f64,
    divisor: u32,
    supported_divisors: Vec<u32>,
    vblank_counter: u64,
}

impl CadenceGovernor {
    pub fn new(display_hz: f64) -> Self {
        let supported_divisors = if display_hz <= 61.0 {
            vec![1, 2]
        } else if display_hz <= 91.0 {
            vec![1, 2, 3]
        } else {
            vec![1, 2, 3, 4]
        };
        Self { display_hz, divisor: 1, supported_divisors, vblank_counter: 0 }
    }

    pub fn supported_divisors(&self) -> &[u32] {
        &self.supported_divisors
    }

    pub fn set_divisor_for_test(&mut self, divisor: u32) {
        self.divisor = divisor;
    }

    pub fn on_vblank(&mut self, _now: Instant) -> CadenceDecision {
        self.vblank_counter += 1;
        let should_present_this_tick = (self.vblank_counter - 1) % self.divisor as u64 == 0;
        let effective_hz = self.display_hz / self.divisor as f64;
        CadenceDecision {
            should_present_this_tick,
            divisor: self.divisor,
            effective_hz,
            target_frame_duration: Duration::from_secs_f64(1.0 / effective_hz),
        }
    }
}
```

- [ ] **Step 4: 导出模块**

在 `crates/dyxel-core/src/lib.rs` 中添加：

```rust
pub mod cadence;
```

- [ ] **Step 5: 运行测试，确认通过**

Run: `cargo test -p dyxel-core cadence_governor_120hz_uses_integer_divisors cadence_governor_only_presents_on_divisor_ticks -- --nocapture`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/dyxel-core/src/lib.rs crates/dyxel-core/src/cadence.rs
git commit -m "feat: add cadence governor with refresh-locked divisor cadence"
```

### Task 3: 引入 FrameTimeline 与新的帧结果分类

**Files:**
- Create: `crates/dyxel-core/src/frame_timeline.rs`
- Modify: `crates/dyxel-render-api/src/lib.rs`
- Modify: `crates/dyxel-core/src/lib.rs`
- Test: `crates/dyxel-core/src/frame_timeline.rs`

- [ ] **Step 1: 写 timeline 记录测试**

```rust
#[test]
fn timeline_records_frame_lifecycle() {
    let mut timeline = FrameTimeline::new();
    let frame_id = timeline.next_frame_id();
    let now = std::time::Instant::now();

    timeline.record_token(frame_id, 7, now, now);
    timeline.mark_render_started(frame_id, now);
    timeline.mark_render_completed(frame_id, now);

    let recent = timeline.recent();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].epoch, 7);
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p dyxel-core timeline_records_frame_lifecycle -- --nocapture`

Expected: FAIL，提示 `FrameTimeline` 未定义

- [ ] **Step 3: 在 render-api 中增加帧结果分类**

在 `crates/dyxel-render-api/src/lib.rs` 中添加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameResultClass {
    OnTime,
    MissedCadence,
    SkippedIdle,
    SkippedDivisor,
    SkippedInFlight,
}
```

- [ ] **Step 4: 实现 `FrameTimeline` 最小版本**

在 `crates/dyxel-core/src/frame_timeline.rs` 中添加：

```rust
use std::collections::VecDeque;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct FrameRecord {
    pub frame_id: u64,
    pub epoch: u64,
    pub token_issued_at: Instant,
    pub vblank_at: Instant,
    pub render_started_at: Option<Instant>,
    pub render_completed_at: Option<Instant>,
}

pub struct FrameTimeline {
    recent: VecDeque<FrameRecord>,
    next_frame_id: u64,
}

impl FrameTimeline {
    pub fn new() -> Self {
        Self { recent: VecDeque::new(), next_frame_id: 1 }
    }

    pub fn next_frame_id(&mut self) -> u64 {
        let id = self.next_frame_id;
        self.next_frame_id += 1;
        id
    }

    pub fn record_token(&mut self, frame_id: u64, epoch: u64, vblank_at: Instant, token_issued_at: Instant) {
        self.recent.push_back(FrameRecord {
            frame_id,
            epoch,
            token_issued_at,
            vblank_at,
            render_started_at: None,
            render_completed_at: None,
        });
    }

    pub fn mark_render_started(&mut self, frame_id: u64, at: Instant) {
        if let Some(rec) = self.recent.iter_mut().find(|r| r.frame_id == frame_id) {
            rec.render_started_at = Some(at);
        }
    }

    pub fn mark_render_completed(&mut self, frame_id: u64, at: Instant) {
        if let Some(rec) = self.recent.iter_mut().find(|r| r.frame_id == frame_id) {
            rec.render_completed_at = Some(at);
        }
    }

    pub fn recent(&self) -> &VecDeque<FrameRecord> {
        &self.recent
    }
}
```

- [ ] **Step 5: 导出模块**

在 `crates/dyxel-core/src/lib.rs` 中添加：

```rust
pub mod frame_timeline;
```

- [ ] **Step 6: 运行测试，确认通过**

Run: `cargo test -p dyxel-core timeline_records_frame_lifecycle -- --nocapture`

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/dyxel-core/src/lib.rs crates/dyxel-core/src/frame_timeline.rs crates/dyxel-render-api/src/lib.rs
git commit -m "feat: add frame timeline and frame result classification"
```

### Task 4: 将 Logic 线程改造成 worker，并保留共享内存桥职责

**Files:**
- Modify: `crates/dyxel-core/src/bridge.rs`
- Modify: `crates/dyxel-core/src/runtime.rs`
- Test: `crates/dyxel-core/src/bridge.rs`

- [ ] **Step 1: 写共享内存桥时序测试**

为 logic worker 增加测试，验证顺序为：

1. `process_commands`
2. `sync_layout_to_wasm`
3. mailbox commit
4. `LogicCommitted`

测试骨架：

```rust
#[test]
fn logic_worker_syncs_layout_before_committing_epoch() {
    // Use a fake mailbox + fake scheduler sender and record call order.
    // Assert layout sync happens before epoch publication.
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p dyxel-core logic_worker_syncs_layout_before_committing_epoch -- --nocapture`

Expected: FAIL，逻辑 worker 尚未抽离

- [ ] **Step 3: 在 `bridge.rs` 中新增 logic worker 命令与循环**

将现有 logic 线程主循环拆成：

- `LogicCommand::ProcessInput`
- `LogicCommand::Tick`
- `LogicCommand::Shutdown`

并保留以下关键顺序：

```rust
process_commands(mem, bptr, &shared_state)?;
sync_layout_to_wasm(mem, bptr, &mut state_guard)?;
let package = runtime_prepare(...);
mailbox.commit(epoch, Arc::new(package));
scheduler_tx.send(SchedulerEvent::LogicCommitted { epoch })?;
```

- [ ] **Step 4: 删除 logic-side 主路径中的 `wait_for_render_or_vsync()`**

从 `crates/dyxel-core/src/bridge.rs` 中移除：

- `render_complete_rx` 作为 logic 节拍源
- `wait_for_render_or_vsync(&render_complete_rx)` 主路径调用

保留其测试或迁移为 scheduler 兼容测试。

- [ ] **Step 5: 保留共享内存边界不变**

确认以下约束：

- `FrameScheduler` 不碰 WASM linear memory
- `Render Worker` 不碰 WASM linear memory
- 只有 logic worker 继续调用 `process_commands(...)` 与 `sync_layout_to_wasm(...)`

- [ ] **Step 6: 运行 bridge/runtime 测试**

Run: `cargo test -p dyxel-core bridge runtime -- --nocapture`

Expected: PASS 或仅出现与旧 `RequestDraw` 路径直接相关的待修测试

- [ ] **Step 7: Commit**

```bash
git add crates/dyxel-core/src/bridge.rs crates/dyxel-core/src/runtime.rs
git commit -m "refactor: convert logic thread into scheduler-driven worker"
```

### Task 5: 将 Render 线程改造成 token 消费者，切断 RequestDraw 主路径

**Files:**
- Modify: `crates/dyxel-core/src/bridge.rs`
- Modify: `crates/dyxel-core/src/pacer.rs`
- Modify: `crates/dyxel-core/src/renderer.rs`
- Modify: `crates/dyxel-render-vello/src/lib.rs`
- Test: `crates/dyxel-core/src/bridge.rs`

- [ ] **Step 1: 写 render worker token 测试**

```rust
#[test]
fn render_worker_only_renders_when_frame_token_arrives() {
    // Feed worker no token and assert no render.
    // Feed one token and assert exactly one render invocation.
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p dyxel-core render_worker_only_renders_when_frame_token_arrives -- --nocapture`

Expected: FAIL，render 线程仍依赖 `RequestDraw/continuous_render`

- [ ] **Step 3: 在 `bridge.rs` 中为 render worker 引入 `RenderCommand::Render(FrameToken)`**

render worker 新入口形态：

```rust
match render_cmd_rx.recv()? {
    RenderCommand::Render(token) => {
        let package = mailbox.snapshot();
        render_frame(..., token, package);
        scheduler_tx.send(SchedulerEvent::RenderCompleted { ... })?;
    }
    RenderCommand::Shutdown => break,
}
```

- [ ] **Step 4: 将 `pacer.rs` 从 render-thread waiter 改成 scheduler-side helper**

保留 `VBlankWaiter` trait，但把“等待下一帧”语义移出 render worker。`pacer.rs` 改造成：

- `VBlankSource` 或等价 helper
- scheduler 使用它产生 `SchedulerEvent::VBlank`

- [ ] **Step 5: 在 `renderer.rs` 中保留 `runtime_prepare()`，但让实际 GPU render 只发生在 render worker**

确保：

- `RenderPackage` 仍由 runtime/logic 产出
- `render_frame()` 不再自己拉起一整轮“prepare + render”
- render worker 只消费已有 package 快照

- [ ] **Step 6: 让 Vello backend 继续只消费 `RenderPackage`**

验证 `dyxel-render-vello` 不重新依赖 `SharedState` 或 WASM memory，只接受：

- `FrameToken` 带来的时序上下文
- `RenderPackage` 快照

- [ ] **Step 7: 运行渲染路径测试**

Run: `cargo test -p dyxel-core render_worker_only_renders_when_frame_token_arrives -- --nocapture`

Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add crates/dyxel-core/src/bridge.rs crates/dyxel-core/src/pacer.rs crates/dyxel-core/src/renderer.rs crates/dyxel-render-vello/src/lib.rs
git commit -m "refactor: make render thread consume frame tokens from scheduler"
```

### Task 6: 启用完整 FrameScheduler 状态机、Governor 与新诊断

**Files:**
- Modify: `crates/dyxel-core/src/frame_scheduler.rs`
- Modify: `crates/dyxel-core/src/cadence.rs`
- Modify: `crates/dyxel-core/src/frame_timeline.rs`
- Modify: `crates/dyxel-render-api/src/lib.rs`
- Modify: `crates/dyxel-render-vello/src/lib.rs`
- Test: `crates/dyxel-core/src/frame_scheduler.rs`
- Test: `crates/dyxel-core/src/cadence.rs`

- [ ] **Step 1: 写 scheduler 状态机测试**

```rust
#[test]
fn scheduler_coalesces_multiple_logic_commits_before_vblank() {
    // Commit epochs 41, 42, 43 before vblank.
    // Assert only 43 gets tokenized.
}

#[test]
fn scheduler_never_issues_second_token_while_render_in_flight() {
    // Put scheduler in Rendering, send another VBlank.
    // Assert no second FrameToken is issued.
}
```

- [ ] **Step 2: 写 governor 升降档测试**

```rust
#[test]
fn governor_downgrades_quickly_on_short_window_pressure() {
    // Feed short-window missed cadence stats on 120Hz and assert divisor 1 -> 2.
}

#[test]
fn governor_upgrades_slowly_after_residency_and_headroom() {
    // Feed stable long-window stats and assert divisor 2 -> 1 only after residency.
}
```

- [ ] **Step 3: 运行测试，确认失败**

Run: `cargo test -p dyxel-core scheduler_coalesces_multiple_logic_commits_before_vblank governor_downgrades_quickly_on_short_window_pressure -- --nocapture`

Expected: FAIL，状态机与 governor 规则尚未完整实现

- [ ] **Step 4: 完整实现 scheduler 事件处理**

在 `crates/dyxel-core/src/frame_scheduler.rs` 中实现：

- `Idle / WaitingForLogic / Armed / Rendering / CoolingDown`
- `InputArrived`
- `LogicCommitted`
- `VBlank`
- `RenderStarted`
- `RenderCompleted`
- `SurfaceChanged`

并确保：

- at most one frame in flight
- latest commit supersedes armed epoch
- render completion 不驱动 logic cadence

- [ ] **Step 5: 完整实现 governor 统计窗口与升降档**

在 `crates/dyxel-core/src/cadence.rs` 中补充：

- short window = 8
- long window = 120
- downgrade pressure = `0.85 * target_frame_ms`
- upgrade headroom = `0.60 * target_frame_ms`
- `min_residency = 2s`

- [ ] **Step 6: 接入新的帧结果分类与 timeline**

让诊断统一使用：

- `OnTime`
- `MissedCadence`
- `SkippedIdle`
- `SkippedDivisor`
- `SkippedInFlight`

并将 timeline 记录接入 backend timing 日志。

- [ ] **Step 7: 运行核心测试与一次项目测试**

Run: `cargo test -p dyxel-core frame_scheduler cadence -- --nocapture`

Run: `cargo test -p dyxel-core -- --nocapture`

Expected: scheduler/cadence 相关测试 PASS；全包测试无新增系统性回归

- [ ] **Step 8: Commit**

```bash
git add crates/dyxel-core/src/frame_scheduler.rs crates/dyxel-core/src/cadence.rs crates/dyxel-core/src/frame_timeline.rs crates/dyxel-render-api/src/lib.rs crates/dyxel-render-vello/src/lib.rs
git commit -m "feat: enable scheduler-centric frame orchestration with adaptive cadence"
```

## Self-Review

### Spec coverage

- 单一 frame owner：Task 4-6 覆盖
- mailbox latest-wins：Task 1 覆盖
- 高刷整数分频：Task 2、Task 6 覆盖
- worker 契约重构：Task 4、Task 5 覆盖
- 共享内存兼容性：Task 4 明确保留
- diagnostics/trace：Task 3、Task 6 覆盖

### Placeholder scan

本计划未使用 `TBD`、`TODO`、`implement later` 等占位词。后续实现若发现接口名不同，应先修正计划再执行。

### Type consistency

计划统一使用以下核心类型名：

- `FrameScheduler`
- `RenderMailbox`
- `CadenceGovernor`
- `FrameTimeline`
- `FrameToken`
- `SchedulerEvent`
- `LogicCommand`
- `RenderCommand`
- `CadenceInfo`
- `LogicFrameHint`

后续执行中不应再引入平行命名。
