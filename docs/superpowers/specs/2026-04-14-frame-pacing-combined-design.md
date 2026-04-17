# Dyxel 帧调度组合修复方案：Atomic Counter Fence + Deadline Lending

**日期**: 2026-04-14  
**目标**: 根治 macOS 上 33.3ms 间歇性跳帧（Lost Wakeup），并建立容错层以平滑 Logic 线程微小抖动。

---

## 1. 问题诊断

当前 DIAG 日志呈现明显规律：
- 正常帧：`PacerWait ≈ 10–12ms`，`FrameInterval ≈ 16.67ms`，`[PERF: OK]`
- JANK 帧：`PacerWait ≈ 27–29ms`，`FrameInterval ≈ 33.3ms`，`[PERF: JANK]`
- `Total` 始终仅 4–6ms，GPU 渲染极快

**根因**：`MacVBlankWaiter` 使用标准库 `Condvar::wait_while` 等待 CVDisplayLink 回调。Condvar 不存储信号：若 callback 的 `notify_one()` 发生在 `wait_while` 开始之前，信号即丢失。Render 线程因此多等待一个完整 VBlank 周期（16.67ms），总间隔变为 33.3ms。

---

## 2. 设计原则：多层防御

| 层级 | 机制 | 解决的问题 |
|------|------|-----------|
| **根治层** | Atomic Counter Fence (macOS) | 消除 `MacVBlankWaiter` 的 lost wakeup |
| **容错层** | Deadline Lending (跨平台) | 当 Logic 微小迟到时，避免惩罚性跳帧 |

---

## 3. 根治层：Atomic Counter Fence for MacVBlankWaiter

### 3.1 文件位置
`mac/src/display_link.rs`

### 3.2 新结构
```rust
struct VBlankState {
    counter: AtomicU64,
    condvar: Condvar,
    mutex: Mutex<()>,
}

pub struct MacVBlankWaiter {
    display_link: CVDisplayLinkRef,
    state: Arc<VBlankState>,
    last_counter: AtomicU64,
}
```

### 3.3 Callback 逻辑
```rust
extern "C" fn display_link_callback(...) -> CVReturn {
    let state = unsafe { &*(context as *const VBlankState) };
    state.counter.fetch_add(1, Ordering::SeqCst);
    let _ = state.condvar.notify_one();
    kCVReturnSuccess
}
```

Callback 仅做两件事：原子计数 + 1，然后 `notify_one()`。

### 3.4 等待逻辑
```rust
impl VBlankWaiter for MacVBlankWaiter {
    fn wait_for_vblank(&self) {
        let start = self.state.counter.load(Ordering::SeqCst);
        let target = start + 1;

        // Fast path: already reached target before blocking
        if self.state.counter.load(Ordering::SeqCst) >= target {
            self.last_counter.store(target, Ordering::SeqCst);
            return;
        }

        // Block with timeout fence to prevent lost-wakeup deadlock
        let guard = self.state.mutex.lock().unwrap();
        let timeout = Duration::from_millis(8);
        let mut guard = guard;
        while self.state.counter.load(Ordering::SeqCst) < target {
            let (new_guard, wait_result) = self.state.condvar.wait_timeout(guard, timeout).unwrap();
            guard = new_guard;
            // If timeout fired, loop back and recheck counter
            if wait_result.timed_out() {
                continue;
            }
        }
        self.last_counter.store(target, Ordering::SeqCst);
    }
}
```

### 3.5 物理防线
- 超时设为 8ms，对于 60Hz（16.67ms 周期），即使 condvar 信号丢失，也最多在周期中点强制醒来重新检查原子计数。
- 由于 `counter` 是原子变量并单调递增，**无论信号是否丢失，Render 线程总能通过重试追上目标**。

---

## 4. 容错层：Deadline Lending in FramePacer

### 4.1 文件位置
`crates/dyxel-core/src/pacer.rs`

### 4.2 核心策略
当 Render 线程在 `wait_for_next_frame` 中被唤醒时，若当前时间已略微超过 target deadline，但仍在“可借用预算”内，则不等下一轮，立即开始渲染。

### 4.3 算法修改
在 `wait_for_next_frame` 末尾、返回之前，加入 Lending 判断：

```rust
const LENDING_BUDGET_MS: f64 = 5.0;

pub fn wait_for_next_frame(&mut self) -> Duration {
    let wait_start = Instant::now();
    // ... existing sleep/spin/VBlank logic ...

    let now = Instant::now();
    let pacer_wait = now.saturating_duration_since(wait_start);

    // ---- Deadline Lending ----
    // If we missed the deadline by a small amount, don't penalize with a full frame skip.
    let lending_budget = Duration::from_secs_f64(LENDING_BUDGET_MS / 1000.0);
    let missed_by = now.saturating_duration_since(self.target_deadline);
    let render_this_frame = if missed_by <= lending_budget {
        // Squeeze the frame in even if slightly late
        true
    } else {
        // Truly missed; accept the frame skip and reset
        false
    };

    // ---- Phase Lock ---- (existing)
    // ...

    // Advance deadline
    let next_deadline = self.target_deadline + self.target_frame_duration;
    self.target_deadline = if next_deadline <= now && !render_this_frame {
        now + self.target_frame_duration
    } else {
        next_deadline
    };

    pacer_wait
}
```

### 4.4 效果
- `now` 在 `target_deadline` 之后 0–5ms：直接渲染，下一帧 deadline 仍正常推进。
- `now` 在 `target_deadline` 之后超过 5ms：判定为严重迟到，reset 到 `now + frame_duration`。
- 从用户感知上，33.3ms 的硬跳帧被转化为 17–21ms 的轻微延迟，几乎不可察觉。

---

## 5. 实施范围

| 文件 | 动作 | 说明 |
|------|------|------|
| `mac/src/display_link.rs` | 重构 | 引入 `Mutex<()>` + `wait_timeout` 的 Atomic Counter Fence |
| `crates/dyxel-core/src/pacer.rs` | 修改 | 加入 `LENDING_BUDGET_MS` 和 Deadline Lending 逻辑 |

### 不改动的文件
- `crates/dyxel-core/src/bridge.rs`：集成点已就位，无需修改。
- `crates/dyxel-render-vello/src/lib.rs`：DIAG 日志已输出所需指标，只需观察验证。

---

## 6. 预期效果

### 修复前（典型 JANK 帧）
```
[DIAG] Frame 35: Total=6.07ms, PacerWait=29.03ms, FrameInterval=33.32ms [PERF: JANK]
```

### 修复后（目标）
```
[DIAG] Frame N: Total=6.1ms, PacerWait=11.2ms, FrameInterval=16.68ms [PERF: OK]
```

- `FrameInterval` 的 33.3ms 尖峰应彻底消失或降至极低频率（< 0.1%）。
- 即使在 Logic 线程出现微小抖动时，也不会触发可见跳帧。

---

## 7. 测试与验证计划

1. **macOS 运行验证**：观察 200 帧以上 DIAG 日志，统计 `[PERF: JANK]` 出现率。
2. **Lost Wakeup 专项**：临时在 `display_link_callback` 中增加日志，确认 `wait_for_vblank` 不再跨越两个 VBlank 周期。
3. **Lending 边界**：在 `pacer.rs` 中临时加入 `trace!` 输出 `missed_by` 毫秒数，确认 Lending 只在 0–3ms 区间内触发。
