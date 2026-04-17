# Frame Pacing Combined Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate 33.3ms intermittent jank on macOS by fixing `MacVBlankWaiter` lost-wakeup with an Atomic Counter Fence, and add cross-platform Deadline Lending to gracefully absorb tiny Logic-thread scheduling jitter.

**Architecture:** Two independent changes: (1) refactor `mac/src/display_link.rs` to use a `Mutex<()>` + `Condvar::wait_timeout` loop around an atomic VBlank counter, and (2) modify `crates/dyxel-core/src/pacer.rs` to allow a 5ms lending budget before forcing a frame skip.

**Tech Stack:** Rust, macOS CoreVideo (CVDisplayLink), std::sync (AtomicU64, Condvar, Mutex)

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `mac/src/display_link.rs` | Modify | `MacVBlankWaiter` atomic-counter fence with timeout |
| `crates/dyxel-core/src/pacer.rs` | Modify | `FramePacer::wait_for_next_frame` Deadline Lending logic |

---

## Task 1: Fix MacVBlankWaiter Lost Wakeup

**Files:**
- Modify: `mac/src/display_link.rs`

### Step 1: Update imports and `VBlankState`

Add `Mutex` to `VBlankState` and change `last_counter` in `MacVBlankWaiter` from `Mutex<u64>` to `AtomicU64`.

```rust
use dyxel_core::pacer::VBlankWaiter;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

// ... callback and structs above unchanged ...

struct VBlankState {
    counter: AtomicU64,
    condvar: Condvar,
    mutex: Mutex<()>,
}
```

### Step 2: Update `MacVBlankWaiter::new` initialization

Replace the `VBlankState` construction so it includes the `mutex`, and change `last_counter` to `AtomicU64::new(0)`.

**Before (around line 107-143):**
```rust
        let state = Arc::new(VBlankState {
            counter: AtomicU64::new(0),
            condvar: Condvar::new(),
        });
```

**After:**
```rust
        let state = Arc::new(VBlankState {
            counter: AtomicU64::new(0),
            condvar: Condvar::new(),
            mutex: Mutex::new(()),
        });
```

And at the end of `new`:

**Before:**
```rust
        Ok(Arc::new(MacVBlankWaiter {
            display_link,
            state,
            last_counter: Mutex::new(0),
        }))
```

**After:**
```rust
        Ok(Arc::new(MacVBlankWaiter {
            display_link,
            state,
            last_counter: AtomicU64::new(0),
        }))
```

### Step 3: Replace `wait_for_vblank` with Atomic Counter Fence

Replace the `VBlankWaiter` impl block entirely.

**Before:**
```rust
impl VBlankWaiter for MacVBlankWaiter {
    fn wait_for_vblank(&self) {
        let mut last = self.last_counter.lock().unwrap();
        let start_counter = self.state.counter.load(Ordering::SeqCst);
        let target = start_counter + 1;
        last = self
            .state
            .condvar
            .wait_while(last, |l| self.state.counter.load(Ordering::SeqCst) < target)
            .unwrap();
        *last = target;
    }
}
```

**After:**
```rust
impl VBlankWaiter for MacVBlankWaiter {
    fn wait_for_vblank(&self) {
        let target = self.state.counter.load(Ordering::SeqCst) + 1;

        // Fast path: already reached target before blocking
        if self.state.counter.load(Ordering::SeqCst) >= target {
            self.last_counter.store(target, Ordering::SeqCst);
            return;
        }

        // Block with timeout fence to prevent lost-wakeup deadlock
        let timeout = Duration::from_millis(8);
        let mut guard = self.state.mutex.lock().unwrap();
        while self.state.counter.load(Ordering::SeqCst) < target {
            let (new_guard, wait_result) = self.state.condvar.wait_timeout(guard, timeout).unwrap();
            guard = new_guard;
            if wait_result.timed_out() {
                continue;
            }
        }
        self.last_counter.store(target, Ordering::SeqCst);
    }
}
```

### Step 4: Verify build

Run:
```bash
./build_mac.sh
```

Expected: compilation succeeds with no new warnings.

### Step 5: Commit

```bash
git add mac/src/display_link.rs
git commit -m "fix(mac): eliminate VBlank lost-wakeup with atomic counter fence

Replaces Condvar::wait_while with a wait_timeout loop around
an atomic VBlank counter. Even if a notification is lost,
the 8ms timeout ensures we recheck the counter mid-period
instead of missing an entire 16.67ms VBlank cycle.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 2: Add Deadline Lending to FramePacer

**Files:**
- Modify: `crates/dyxel-core/src/pacer.rs`

### Step 1: Add lending constant

Insert near the top of `pacer.rs`, after the existing constants:

```rust
const LENDING_BUDGET_MS: u64 = 5; // Allow up to 5ms past deadline before skipping a frame
```

### Step 2: Inject Deadline Lending into `wait_for_next_frame`

Locate the comment `// Robust reset:` (around line 130). Insert the lending logic just before it.

**Before:**
```rust
        // Apply correction: if we're consistently late (drift > 0), pull the deadline earlier.
        // If we're consistently early (drift < 0), push it later.
        let next_deadline = if correction_secs > 0.0 {
            self.target_deadline + self.target_frame_duration - correction
        } else {
            self.target_deadline + self.target_frame_duration + correction
        };

        // Robust reset: if deadline has already passed, start fresh from now.
        self.target_deadline = if next_deadline <= now {
            now + self.target_frame_duration
        } else {
            next_deadline
        };
```

**After:**
```rust
        // ---- Deadline Lending ----
        // If we missed the deadline by only a small amount, render this frame anyway
        // instead of punishing with a full frame skip.
        let missed_by = now.saturating_duration_since(self.target_deadline);
        let lending_budget = Duration::from_millis(LENDING_BUDGET_MS);
        let render_this_frame = missed_by <= lending_budget;

        // Apply correction: if we're consistently late (drift > 0), pull the deadline earlier.
        // If we're consistently early (drift < 0), push it later.
        let next_deadline = if correction_secs > 0.0 {
            self.target_deadline + self.target_frame_duration - correction
        } else {
            self.target_deadline + self.target_frame_duration + correction
        };

        // Robust reset: if deadline has already passed, start fresh from now.
        // Lending exception: if we're only slightly late, squeeze the frame in.
        self.target_deadline = if next_deadline <= now && !render_this_frame {
            now + self.target_frame_duration
        } else {
            next_deadline
        };
```

### Step 3: Verify build

Run:
```bash
./build_mac.sh
```

Expected: compilation succeeds with no new warnings.

### Step 4: Commit

```bash
git add crates/dyxel-core/src/pacer.rs
git commit -m "feat(pacer): add 5ms Deadline Lending to avoid punitive frame skips

If the render thread wakes slightly past the target deadline
(<= 5ms), we render the frame immediately instead of forcing
a full 16.67ms skip. This smooths tiny Logic-thread jitter
into imperceptible delays.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 3: Runtime Verification on macOS

**Files:** None (observation only)

### Step 1: Run the app and observe DIAG logs

Run:
```bash
./build_mac.sh
```

Then launch the built binary (or however the project is started locally). Watch the console output for `[DIAG]` lines.

### Step 2: Check JANK rate

Look at ~200 consecutive frames. The acceptance criteria are:
- No `FrameInterval ≈ 33.3ms` spikes.
- `[PERF: JANK]` appears in < 1% of frames (ideally 0%).
- `PacerWait` on OK frames stays in the 10–14ms range.

### Step 3: Commit observation notes (optional)

If you capture a representative log snippet, paste it into a comment or note. No code commit is required if only running.

---

## Spec Coverage Checklist

| Spec Section | Plan Task |
|--------------|-----------|
| 3.1 File location (`mac/src/display_link.rs`) | Task 1 |
| 3.2 New `VBlankState` with `Mutex<()>` | Task 1, Step 1 |
| 3.3 Callback logic (atomic increment + notify) | Task 1, Step 3 (preserved) |
| 3.4 `wait_timeout` loop in `wait_for_vblank` | Task 1, Step 3 |
| 4.1 File location (`crates/dyxel-core/src/pacer.rs`) | Task 2 |
| 4.3 Lending algorithm (`missed_by`, `lending_budget`, `render_this_frame`) | Task 2, Step 2 |
| 6 Expected effect (eliminate 33.3ms spikes) | Task 3 |
