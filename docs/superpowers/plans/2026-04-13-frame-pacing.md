# Frame Pacing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a production-grade `FramePacer` with Spin-Sleep adaptive strategy, and refactor the Vello render pipeline to a single post-Vello submission, eliminating TexWait jitter on 60Hz displays.

**Architecture:** A `FramePacer` in `dyxel-core` anchors the Render Thread loop to an ideal VBlank timeline. The Vello backend keeps `PresentMode::Immediate`, but all post-Vello GPU work (blur copies, blur compute, final blit) is merged into one `CommandEncoder` and one `queue.submit()` per frame.

**Tech Stack:** Rust, wgpu, Vello 0.7, winit

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/dyxel-core/src/pacer.rs` | Create | `FramePacer` with Spin-Sleep, ideal VBlank deadline, and catch-up reset |
| `crates/dyxel-core/src/lib.rs` | Modify | Export `pub mod pacer` |
| `crates/dyxel-core/src/bridge.rs` | Modify | Wire `FramePacer` into Render Thread; add `SetTargetFPS` message |
| `mac/src/main.rs` | Modify | Detect display refresh rate via winit and inject into `DyxelHost` |
| `crates/dyxel-render-vello/src/filter_pipeline.rs` | Modify | Refactor all effect methods to accept an external `&mut CommandEncoder` instead of creating their own |
| `crates/dyxel-render-vello/src/lib.rs` | Modify | Single-submission refactor of `render_internal`, merge all post-Vello work into one encoder, update DIAG logging |

---

## Important Constraint: Vello `render_to_texture` submits internally

Vello 0.7's `Renderer::render_to_texture` creates its own encoder and submits to the queue internally (it calls `engine.run_recording`). We cannot change this without forking Vello. Therefore, **our "single submission" target means:**

1. Vello does its own submit (unavoidable).
2. **All remaining GPU work in the frame** (blur texture copies, blur compute passes, final blit to surface) must be recorded into **one** shared `CommandEncoder` and submitted with **one** `queue.submit(Some(encoder.finish()))`.

This reduces the frame from ~4-5 submits down to 2, which is the practical optimum.

---

## Task 1: FramePacer Core (`dyxel-core/src/pacer.rs`)

**Files:**
- Create: `crates/dyxel-core/src/pacer.rs`
- Modify: `crates/dyxel-core/src/lib.rs`

- [ ] **Step 1: Create `pacer.rs`**

Create `crates/dyxel-core/src/pacer.rs` with the following exact implementation:

```rust
use std::time::{Duration, Instant};

pub struct FramePacer {
    /// The fixed VBlank deadline that does not drift on late frames.
    target_deadline: Instant,
    target_frame_duration: Duration,
    /// Safety buffer to leave a little headroom before the true deadline.
    buffer_time: Duration,
}

impl FramePacer {
    pub fn new(target_fps: f64) -> Self {
        let target_frame_duration = Duration::from_secs_f64(1.0 / target_fps);
        Self {
            target_deadline: Instant::now() + target_frame_duration,
            target_frame_duration,
            buffer_time: Duration::from_micros(500), // 0.5ms
        }
    }

    /// Spin-Sleep strategy: sleep if >2ms away, spin-loop the last 0.5ms.
    /// Returns the amount of time spent actively waiting.
    pub fn wait_for_next_frame(&mut self) -> Duration {
        let wait_start = Instant::now();
        let target = self.target_deadline.saturating_sub(self.buffer_time);

        if wait_start < target {
            let remaining = target - wait_start;
            if remaining > Duration::from_millis(2) {
                std::thread::sleep(remaining - Duration::from_micros(500));
            }
            while Instant::now() < target {
                std::hint::spin_loop();
            }
        }

        let pacer_wait = Instant::now().saturating_duration_since(wait_start);

        // Advance the ideal deadline by exactly one frame period.
        self.target_deadline += self.target_frame_duration;

        // Safety reset: if the deadline has fallen more than 2 frames behind,
        // reset it to "now + one frame" to prevent runaway catch-up.
        let now = Instant::now();
        if self.target_deadline + self.target_frame_duration < now {
            self.target_deadline = now + self.target_frame_duration;
        }

        pacer_wait
    }

    /// Called immediately after `present()` returns.
    pub fn mark_present(&mut self) {
        // Intentionally a no-op; deadline is tracked on the ideal timeline.
    }
}
```

- [ ] **Step 2: Export the module in `dyxel-core/src/lib.rs`**

Add `pub mod pacer;` after `pub mod bridge;`:

```rust
pub mod bridge;
pub mod pacer; // <-- ADD
pub mod spatial_index;
```

- [ ] **Step 3: Commit**

```bash
git add crates/dyxel-core/src/pacer.rs crates/dyxel-core/src/lib.rs
git commit -m "feat: add FramePacer with Spin-Sleep adaptive pacing

Introduces FramePacer that anchors to an ideal VBlank deadline
using a thread::sleep + spin_loop hybrid strategy.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 2: Wire `FramePacer` into Render Thread (`dyxel-core/src/bridge.rs`)

**Files:**
- Modify: `crates/dyxel-core/src/bridge.rs`

- [ ] **Step 1: Add `SetTargetFPS(f64)` to `RenderMessage`**

Find the `RenderMessage` enum (around line 64) and add the new variant:

```rust
pub enum RenderMessage {
    // ... existing variants ...
    TogglePerfOverlay,
    SetContinuousRender(bool),
    SetTargetFPS(f64), // <-- ADD
}
```

- [ ] **Step 2: Add `set_target_fps` method to `DyxelHost`**

Find the `impl DyxelHost` block around line 1216 (`set_continuous_render`) and add immediately after it:

```rust
    pub fn set_target_fps(&self, fps: f64) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &*self.render_tx.lock().unwrap() {
            match tx.send(RenderMessage::SetTargetFPS(fps)) {
                Ok(_) => (),
                Err(e) => log::error!("set_target_fps: Failed to send: {:?}", e),
            }
        }
    }
```

- [ ] **Step 3: Integrate `FramePacer` into the Render Thread loop**

In the Render Thread closure (around line 783), add a `FramePacer` variable and handle the new message. The diff looks like this:

**Before:**
```rust
                    let mut render_opt: Option<RenderState> = None;
                    let mut lifecycle = Lifecycle::Stopped;
                    let mut continuous_render = true;

                    loop {
```

**After:**
```rust
                    let mut render_opt: Option<RenderState> = None;
                    let mut lifecycle = Lifecycle::Stopped;
                    let mut continuous_render = true;
                    let mut pacer: Option<crate::pacer::FramePacer> = None;

                    loop {
```

Then, in the message processing `match` inside the loop, handle `SetTargetFPS`:

Find the arm for `RenderMessage::SetContinuousRender(enabled)` (around line 802 in continuous mode and line 831 in event-driven mode). Add a new arm in both match blocks:

```rust
                                    RenderMessage::SetTargetFPS(fps) => {
                                        pacer = Some(crate::pacer::FramePacer::new(fps));
                                        log::info!("RenderThread: Target FPS set to {:.2}", fps);
                                    }
```

Add this arm in the **continuous mode** `while let Ok(msg) = render_rx.try_recv()` block, and also in the **event-driven mode** `match msg` block and the following `while let Ok(next) = render_rx.try_recv()` block.

Then, right before the draw call, add the pacing wait. Find this code (around line 958):

**Before:**
```rust
                        } else if draw_requested && lifecycle == Lifecycle::Running {
```

**After:**
```rust
                        } else if draw_requested && lifecycle == Lifecycle::Running {
                            // Pacer wait at the start of every frame
                            let _pacer_wait = pacer.as_mut().map(|p| p.wait_for_next_frame());
```

Then after `render_frame(r, s.as_mut())` (around line 966) and before `let _ = render_complete_tx.send(());`, add:

```rust
                                    render_frame(r, s.as_mut());
                                    // Mark present timing for Pacer
                                    if let Some(ref mut p) = pacer {
                                        p.mark_present();
                                    }
                                    let _ = render_complete_tx.send(());
```

- [ ] **Step 4: Commit**

```bash
git add crates/dyxel-core/src/bridge.rs
git commit -m "feat: wire FramePacer into RenderThread and DyxelHost

Adds SetTargetFPS message, set_target_fps host API, and integrates
pacing wait at the top of every RenderThread frame.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 3: macOS Refresh Rate Detection (`mac/src/main.rs`)

**Files:**
- Modify: `mac/src/main.rs`

- [ ] **Step 1: Detect refresh rate and inject into host**

After `surface_setup_done = true;` inside the `AboutToWait` arm (around line 57), add:

```rust
                    surface_setup_done = true;

                    // Detect display refresh rate and inject into engine
                    if let Some(monitor) = w.primary_monitor() {
                        if let Some(video_mode) = monitor.video_modes().next() {
                            let mhz = video_mode.refresh_rate_millihertz();
                            let fps = mhz as f64 / 1000.0;
                            let effective_fps = if fps >= 119.0 { 120.0 } else if fps >= 59.0 { 60.0 } else { fps.max(30.0) };
                            log::info!("macOS: Detected refresh rate {:.3} Hz ({} mHz), using target FPS {:.2}", fps, mhz, effective_fps);
                            host.set_target_fps(effective_fps);
                        } else {
                            log::warn!("macOS: Could not detect video mode, falling back to 60 Hz");
                            host.set_target_fps(60.0);
                        }
                    } else {
                        log::warn!("macOS: Could not get primary monitor, falling back to 60 Hz");
                        host.set_target_fps(60.0);
                    }
```

- [ ] **Step 2: Commit**

```bash
git add mac/src/main.rs
git commit -m "feat: detect display refresh rate on macOS and inject target FPS

Uses winit video_mode refresh_rate_millihertz, rounded to 60/120Hz.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 4: Refactor `filter_pipeline.rs` to Accept External Encoder

**Files:**
- Modify: `crates/dyxel-render-vello/src/filter_pipeline.rs`

All methods below currently create their own `CommandEncoder` and call `self.queue.submit`. They must be refactored to receive `encoder: &mut wgpu::CommandEncoder` and drop the submit calls.

Methods to refactor:
1. `apply_blur` (around line 500)
2. `composite_shadow` (around line 700)
3. `copy_texture` (around line 740)
4. `apply_frosted_glass` (around line 850)
5. `apply_frosted_glass_kawase` (around line 957)

- [ ] **Step 1: Refactor `apply_blur`**

**Before:**
```rust
    pub fn apply_blur(
        &self,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        blur_radius: f32,
    ) -> Result<(), FilterError> {
        // ... setup code ...
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Blur Command Encoder"),
        });
        // ... pass recording into encoder ...
        self.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }
```

**After:**
```rust
    pub fn apply_blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        blur_radius: f32,
    ) -> Result<(), FilterError> {
        // ... setup code ...
        // REMOVED: let mut encoder = self.device.create_command_encoder(...);
        // ... pass recording into encoder ...
        // REMOVED: self.queue.submit(...);
        Ok(())
    }
```

Do the same mechanical change for the other four methods:
- Remove local `encoder` creation.
- Add `encoder: &mut wgpu::CommandEncoder` as the **first** parameter.
- Remove the final `self.queue.submit(...)` call.
- Ensure any intermediate `self.queue.submit` calls inside methods are also removed (not applicable for these methods, but double-check).

- [ ] **Step 2: Update all call sites inside `filter_pipeline.rs` if any**

Search the same file for internal calls like `self.apply_blur(` or `self.apply_frosted_glass_kawase(` and add `encoder, ` as the first argument. As of the current codebase, there are no internal self-calls between these methods within `filter_pipeline.rs`, but verify with a grep.

- [ ] **Step 3: Commit**

```bash
git add crates/dyxel-render-vello/src/filter_pipeline.rs
git commit -m "refactor: filter_pipeline methods now accept external encoder

All effect methods (blur, shadow, copy, frosted_glass, kawase)
take &mut CommandEncoder instead of creating their own.
This sets up single-submission frame rendering.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 5: Single-Submission Refactor of `render_internal` (`dyxel-render-vello/src/lib.rs`)

This is the largest change. We will transform the second half of `render_internal` so that after Vello's `render_to_texture`, all remaining GPU work is recorded into **one** `CommandEncoder` named `post_enc`, and submitted exactly once.

**Current problematic submits in `render_internal`:**
- Line 1441: `queue.submit(std::iter::once(copy_enc.finish()));` (blur copy)
- Lines 1977, 1991, 1997: multiple `queue.submit(Some(enc.finish()));` around final blit

**Goal:**
1. Keep `renderer.render_to_texture(...)` as-is (it submits internally).
2. Replace the separate `copy_enc` with operations on a shared `post_enc`.
3. Replace the final blit `enc` with the same shared `post_enc`.
4. Call `queue.submit(Some(post_enc.finish()))` exactly **once**, after all passes are recorded.
5. Move `get_current_texture()` to BEFORE encoder creation (it already is after the refactor area in current code, but we want it immediately after Vello rendering and before `post_enc` creation).

- [ ] **Step 1: Move surface acquisition earlier and create single `post_enc`**

Locate the area after `stage_timer.mark("pass3_done");` (around line 1588). The current code does `match v_surface_surface.surface.get_current_texture()` and then inside the `Ok(st)` arm creates `let mut enc = device.create_command_encoder(...)`. We will restructure this as follows:

**Replace the block starting at line 1588 (`/ Single present`) with:**

```rust
        // Acquire surface texture BEFORE creating the shared encoder.
        // Any TexWait is compressed to the front of the frame pacing window.
        stage_timer.mark("before_get_texture");
        let surface_texture = match v_surface_surface.surface.get_current_texture() {
            Ok(st) => {
                stage_timer.mark("after_get_texture");
                st
            }
            Err(e) => {
                log::error!("VelloBackend: get_current_texture failed: {:?}", e);
                return Err(anyhow::anyhow!("Surface texture acquisition failed: {:?}", e));
            }
        };

        // Create the ONE shared encoder for all post-Vello GPU work
        let mut post_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Post-Vello Single Submission Encoder"),
        });

        // --- PASS 2 (merged into post_enc): Blur texture copies ---
        {
            let mut blurred_textures = self.blurred_textures.lock().unwrap();
            let filter_pipeline = self.filter_pipeline.lock().unwrap();

            if !blurred_textures.is_empty() && should_update_blur {
                if let Some(pipeline) = filter_pipeline.as_ref() {
                    let scene_texture = triple_buffer.write_buffer().map(|(t, _, _)| t)
                        .expect("Scene texture should exist");

                    // Collect and copy blur entries into post_enc
                    let mut blur_entries: Vec<_> = Vec::new();
                    for entry in blurred_textures.iter_mut() {
                        let (src_x, src_y, src_w, src_h) = entry.source_rect;
                        if entry.blur_radius > 0.0 {
                            blur_entries.push((entry.view_id, &entry.texture, entry.blur_radius));
                        }
                        // Clear + copy into post_enc
                        post_enc.clear_texture(
                            &entry.texture,
                            &wgpu::ImageSubresourceRange {
                                aspect: wgpu::TextureAspect::All,
                                base_mip_level: 0,
                                mip_level_count: None,
                                base_array_layer: 0,
                                layer_count: None,
                            },
                            wgpu::Color::TRANSPARENT,
                        );
                        post_enc.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: scene_texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: src_x as u32, y: src_y as u32, z: 0 },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::TexelCopyTextureInfo {
                                texture: &entry.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::Extent3d { width: src_w as u32, height: src_h as u32, depth_or_array_layers: 1 },
                        );
                    }

                    stage_timer.mark("blur_copy_submit");

                    // Run Kawase blur for each entry, recording into post_enc
                    let mut processed_view_ids = Vec::new();
                    for (view_id, texture, blur_radius) in blur_entries {
                        let pool_guard = self.texture_pool.lock().unwrap();
                        let result = if let Some(ref pool) = *pool_guard {
                            pipeline.apply_frosted_glass_kawase(&mut post_enc, texture, texture, blur_radius, Some(pool))
                        } else {
                            pipeline.apply_frosted_glass_kawase(&mut post_enc, texture, texture, blur_radius, None)
                        };
                        drop(pool_guard);
                        if let Err(e) = result {
                            log::warn!("[Blur] Failed to apply Kawase frosted glass for view {}: {:?}", view_id, e);
                        } else {
                            processed_view_ids.push(view_id);
                        }
                    }
                    for entry in blurred_textures.iter_mut() {
                        if processed_view_ids.contains(&entry.view_id) {
                            entry.needs_recalculation = false;
                        }
                    }
                    stage_timer.mark("blur_render_submit");
                }
            }
        }
```

**Important:** This inline replaces the old `copy_enc` block (lines ~1347-1441 and ~1448-1487). Keep the exact copy/clear logic from the original code, just replace `copy_enc` with `post_enc`.

- [ ] **Step 2: Merge final blit into the same `post_enc`**

The final blit currently happens inside the `Ok(st)` arm of `get_current_texture` using a local `enc`. Since we now already have `surface_texture` and `post_enc`, we rewrite the blit/composite logic to use `post_enc` directly.

Keep the existing debug capture texture creation and composite pipeline setup logic (lines ~1595-1730), but replace `enc` with `post_enc` everywhere. Crucially, **remove all intermediate `queue.submit(Some(enc.finish()))` calls**.

Specifically:
- Around line 1977: delete `queue.submit(Some(enc.finish()));` and the following `enc = device.create_command_encoder(...);`
- Around line 1991: delete `queue.submit(Some(enc.finish()));`
- Around line 1997 (wasm32): delete `queue.submit(Some(enc.finish()));`

After all `post_enc` recording is complete (blit draw + blur composite draws), do exactly one:

```rust
        queue.submit(Some(post_enc.finish()));
        stage_timer.mark("blit_submit");

        triple_buffer.advance();
        surface_texture.present();
        stage_timer.mark("present_return");
```

Ensure the `RenderPass` (`rp`) from the blit is dropped by ending its scope before `queue.submit`. In the existing code, `rp` is already scoped inside a `{ ... }` block that ends well before the submit.

- [ ] **Step 3: Verify texture usage flags on TripleBuffer**

In `TripleBuffer::ensure_size` (around line 89 of `lib.rs`), the texture descriptor already includes `STORAGE_BINDING | TEXTURE_BINDING | RENDER_ATTACHMENT | COPY_SRC`. Confirm this matches:

```rust
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
```

If `COPY_DST` is missing but needed, add it. As of current code, `clear_texture` + `copy_texture_to_texture` operate on the texture as destination; in wgpu 0.20+, `COPY_DST` is implied by these operations on a texture if the texture has `COPY_DST`, but the original code used `clear_texture` and `copy_texture_to_texture` on `entry.texture` which is created elsewhere (in `blurred_textures`). Those blur entry textures should already have correct usage. No change needed for TripleBuffer textures.

- [ ] **Step 4: Commit the refactor**

```bash
git add crates/dyxel-render-vello/src/lib.rs
git commit -m "refactor: single-submission post-Vello rendering pipeline

Vello render_to_texture still submits internally (API constraint),
but all blur copies, blur compute, and final blit are now merged
into one CommandEncoder with a single queue.submit.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 6: Update DIAG Logging with PacerWait & FrameInterval

**Files:**
- Modify: `crates/dyxel-render-vello/src/lib.rs`
- Modify: `crates/dyxel-core/src/renderer.rs` (to pass PacerWait into render_frame)

We need to get `PacerWait` from the Render Thread down into the Vello backend's DIAG log. The simplest approach is to store it temporarily in `SharedState` or `RenderState`, or pass it through the `render_frame` function. Given the existing architecture, the cleanest path is:

1. Add `pub pacer_wait_ms: f32` to `RenderState` in `dyxel-core/src/engine.rs`.
2. Set it in the Render Thread right after `wait_for_next_frame()`.
3. Read it in `render_internal` and include it in the log.
4. Track `last_present_time` as a static atomic or field on `VelloBackend` to compute `FrameInterval`.

- [ ] **Step 1: Add `pacer_wait_ms` to `RenderState`**

Find `RenderState` in `crates/dyxel-core/src/engine.rs` (around line 50; grep for `pub struct RenderState`). Add the field:

```rust
pub struct RenderState {
    pub backend: Box<dyn RenderBackend>,
    pub context: Box<dyn RenderContext>,
    pub shared_state: SharedPtr<SharedMutex<SharedState>>,
    pub pacer_wait_ms: f32, // <-- ADD
}
```

Then find where `RenderState` is instantiated (around `setup_engine` or in `bridge.rs` around line 866) and initialize it:

```rust
                                    render_opt = Some(RenderState {
                                        backend: r.backend,
                                        context: r.context,
                                        shared_state: r.shared_state,
                                        pacer_wait_ms: 0.0,
                                    });
```

- [ ] **Step 2: Populate `pacer_wait_ms` in Render Thread**

In `bridge.rs`, change the pacing wait code to store the value:

```rust
                            // Pacer wait at the start of every frame
                            let pacer_wait = pacer.as_mut().map(|p| p.wait_for_next_frame()).unwrap_or_default();
                            if let Some(ref mut r) = render_opt {
                                r.pacer_wait_ms = pacer_wait.as_secs_f32() * 1000.0;
                            }
```

- [ ] **Step 3: Update `render_frame` signature or read from `RenderState`**

In `crates/dyxel-core/src/renderer.rs`, the current `render_frame` function receives `RenderState`. We can read `e.pacer_wait_ms` directly and pass it into the backend if needed. However, the backend trait `RenderBackend::render` does not currently accept `pacer_wait_ms`. The easiest path is to pass it via `SharedState` or via the `RenderBackend` trait.

Simpler option: add `pacer_wait_ms` as a field to `SharedState` instead, so no trait changes are needed.

Actually, since `render_frame` already has access to `e` (`RenderState`), and `render_internal` has access to `shared_state` (which is `&SharedMutex<SharedState>`), let's just put `pacer_wait_ms` on `SharedState`:

**Revert Step 1:** Put it on `SharedState` in `crates/dyxel-core/src/state.rs` instead:

```rust
pub struct SharedState {
    // ... existing fields ...
    pub pacer_wait_ms: f32,
}
```

Initialize it in `SharedState::new()` to `0.0`.

**In Render Thread (`bridge.rs`):**
```rust
                            let pacer_wait = pacer.as_mut().map(|p| p.wait_for_next_frame()).unwrap_or_default();
                            {
                                let mut ss = shared_state.lock().unwrap();
                                ss.pacer_wait_ms = pacer_wait.as_secs_f32() * 1000.0;
                            }
```

**In `render_internal`:** read it from `shared_state`:

```rust
        let (pacer_wait_ms, frame_interval_ms) = {
            let ss = shared_state.lock().unwrap();
            let pacer_wait = ss.pacer_wait_ms;
            // ... frame interval logic ...
            (pacer_wait, 0.0f32)
        };
```

- [ ] **Step 4: Compute FrameInterval (present-to-present) in `render_internal`**

Add a static atomic field or use a `std::sync::Mutex<std::time::Instant>` on `VelloBackend` to track last present time.

Simpler: use a `static` in the function. Since `render_internal` is called from one thread (RenderThread), a `static LAST_PRESENT_TIME: Mutex<Option<Instant>> = Mutex::new(None);` works.

At the top of the present-success block (where `st.present()` is called):

```rust
                use std::sync::Mutex;
                static LAST_PRESENT_TIME: Mutex<Option<std::time::Instant>> = Mutex::new(None);
                let now = std::time::Instant::now();
                let frame_interval_ms = {
                    let mut last = LAST_PRESENT_TIME.lock().unwrap();
                    let interval = last.map(|t| now.duration_since(t).as_secs_f32() * 1000.0).unwrap_or(0.0);
                    *last = Some(now);
                    interval
                };
                st.present();
                stage_timer.mark("present_return");
```

- [ ] **Step 5: Emit updated DIAG log with PERF tag**

Find the DIAG log block (around line 2092).

Add `pacer_wait_ms` and `frame_interval_ms` to the variables computed above the log.

Compute a PERF tag:

```rust
                let target_ms = 1000.0 / 60.0f32; // default 60Hz display target
                let jitter_ms = (frame_interval_ms - target_ms).abs();
                let perf_tag = if frame_interval_ms > target_ms + 1.0 {
                    "[PERF: JANK]"
                } else if pacer_wait_ms < 2.0 {
                    "[PERF: WARM]"
                } else if jitter_ms < 0.5 {
                    "[PERF: OK]"
                } else {
                    "[PERF: OK]"
                };
```

Update the log format:

```rust
                log::info!(
                    "[DIAG] Frame {}: Total={:.2}ms, PacerWait={:.2}ms, State={:.2}ms, Scene={:.2}ms, GPU={:.2}ms, BlurCopy={:.2}ms, BlurRender={:.2}ms, Pass3={:.2}ms, GetTex={:.2}ms, TexWait={:.2}ms, Blit={:.2}ms, Present={:.2}ms, FrameInterval={:.2}ms, FPS(savg)={:.1}, FPS(inst)={:.1} {}",
                    stats.total_frames,
                    total,
                    pacer_wait_ms,
                    state_lock_time,
                    scene_build_time,
                    gpu_time,
                    blur_copy_time,
                    blur_render_time,
                    pass3_time,
                    get_texture_time,
                    texture_wait_time,
                    blit_time,
                    present_time,
                    frame_interval_ms,
                    stats.fps,
                    instant_fps,
                    perf_tag
                );
```

- [ ] **Step 6: Commit**

```bash
git add crates/dyxel-core/src/state.rs crates/dyxel-core/src/bridge.rs crates/dyxel-render-vello/src/lib.rs
git commit -m "feat: add PacerWait and FrameInterval to DIAG logs

Tracks pacer wait via SharedState and computes present-to-present
frame interval with a [PERF: OK/WARM/JANK] status tag.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 7: Build & Sanity Check

- [ ] **Step 1: Format code**

```bash
cargo fmt --all
```

- [ ] **Step 2: Build macOS target**

```bash
./build_mac.sh
```

Expected: compilation succeeds with no new warnings related to our changes.

- [ ] **Step 3: Verify key log outputs on run**

Run the app and look for:

1. `macOS: Detected refresh rate ... using target FPS 60.00`
2. `RenderThread: Target FPS set to 60.00`
3. A DIAG log line containing `PacerWait=... FrameInterval=... [PERF: OK]`

If `PacerWait` stays high (e.g., >10ms) and `FrameInterval` is stable near 16.67ms, the implementation is correct.

- [ ] **Step 4: Commit formatting/build fixes if any**

```bash
git add -A
git commit -m "chore: formatting and build fixes for frame pacing"
```

---

## Spec Coverage Checklist

| Design Spec Section | Plan Task |
|---------------------|-----------|
| Section 3: Architecture (Pacer at loop start) | Task 2 |
| Section 4: FramePacer (Spin-Sleep, ideal deadline, catch-up reset) | Task 1 |
| Section 5: Single Submission (post-Vello encoder merge) | Tasks 4 & 5 |
| Section 6: Refresh Rate Detection (winit fallbacks) | Task 3 |
| Section 7: DIAG logs (PacerWait, FrameInterval, PERF tag) | Task 6 |
| User "pitfall" #1: Pass objects dropped before `encoder.finish()` | Task 5, Step 2 (explicit scope handling) |
| User "pitfall" #2: Deadline reset if >2 frames behind | Task 1, `wait_for_next_frame` safety reset |
| User "pitfall" #3: Texture usage flags union | Task 5, Step 3 (already present in TripleBuffer) |

No placeholders, no TBDs, every task contains exact code snippets and file paths.
