# Vello Render Performance Optimization Design

> **Goal:** Eliminate per-frame full-scene CPU rebuild and GPU resource churn in the Vello render backend, focusing on incremental layout/scene caching, blur texture lifecycle fixes, and bind-group reuse.
> **Version:** 2.0 (revised after code-level feasibility review)

---

## 1. Background & Problem Statement

Current profiling shows the render backend spends significant time on:

1. **CPU-side full rebuild:** Every frame re-measures all text nodes, re-runs `compute_layout_with_measure()`, and recursively rebuilds the entire `Scene` from the root (`lib.rs:1051-1217`).
2. **Blur texture waste:** `render_with_blur()` creates a new `wgpu::Texture` on every call even when the cached entry is reused (`lib.rs:2306-2326`).
3. **Full-screen children pass:** When any blur node exists, Pass 3 renders deferred children to a **fullscreen** texture (`lib.rs:1634-1715`), burning bandwidth regardless of actual blur card size.
4. **GPU object churn:** Blur composite and Kawase passes recreate `TextureView` + `BindGroup` per entry per pass (`lib.rs:1976-2019`, `filter_pipeline.rs:1033-1165`).

---

## 2. Scope

This design covers **four independent optimization work-streams** in `crates/dyxel-render-vello`:

| # | Work-Stream | Primary Files |
|---|---|---|
| 1 | Fix Invalid `create_texture` in `render_with_blur` | `lib.rs:2280-2430` |
| 2 | Reuse Blur Composite / Kawase TextureViews | `lib.rs:1976-2019`, `filter_pipeline.rs:1033-1165` |
| 3 | Replace Full-Screen Children Pass with Local Rendering | `lib.rs:1634-1715`, `lib.rs:2488-2529` |
| 4 | Incremental Scene / Layout + Raster Cache Integration | `lib.rs:1051-1217`, `raster_cache.rs`, `layer.rs` |

**Out of scope:**
- Removing the 33 ms logic-thread boundary wait (`bridge.rs:27/775`) — input-latency improvement, not directly FPS.
- Adding a "no-effects direct-to-surface" fast path — valuable but secondary to the P0 items above.
- Cleaning debug/info logs in blur paths — trivial, can be done opportunistically.

---

## 3. Design Details

### 3.1 Fix Invalid `create_texture` in `render_with_blur` (Task A)

**Current behavior:**
- `render_with_blur()` unconditionally calls `device.create_texture(&texture_desc)` at `lib.rs:2326`.
- If an entry already exists for this `view_id` and the size hasn't changed, the newly created texture is simply dropped (overwritten at `lib.rs:2406` or never used at `lib.rs:2410`).

**Desired behavior:**
- Only create the texture when we actually need a new one (new entry or size changed).

**Implementation approach:**

```rust
let existing_index = blurred_textures.iter().position(|e| e.view_id == id);
let needs_new_texture = existing_index.map_or(true, |idx| {
    let entry = &blurred_textures[idx];
    entry.width != texture_width || entry.height != texture_height
});

let offscreen_texture = if needs_new_texture {
    Some(device.create_texture(&texture_desc))
} else {
    None
};
```

Then in the update path:
```rust
if let Some(tex) = offscreen_texture {
    entry.texture = tex;
    entry.width = texture_width;
    entry.height = texture_height;
}
```

And in the new-entry path:
```rust
blurred_textures.push(BlurredTextureEntry {
    texture: offscreen_texture.expect("new entry must have texture"),
    // ...
});
```

**Validation:** Profile or log `device.create_texture` calls inside `render_with_blur`; should drop to zero for stable blur nodes.

---

### 3.2 Reuse Blur Composite / Kawase TextureViews (Task B)

**Current behavior:**
- In the blur composite loop (`lib.rs:1976-2019`), each entry does:
  ```rust
  let texture_view = entry.texture.create_view(...);
  let bind_group = device.create_bind_group(...);
  ```
- In `filter_pipeline.rs:1033-1165`, each Kawase pass calls `create_view(&Default::default())` on intermediate pool textures every pass.

**Desired behavior:**
- Avoid per-frame `create_view` for stable resources.
- **Note:** Full `BindGroup` reuse across frames is *not* a quick fix because the blur composite pipeline uses two uniform buffer bindings with `has_dynamic_offset: false` (`lib.rs:814-850`). Converting them to dynamic offsets requires pipeline-layout, shader, and draw-call changes. Task B therefore targets `TextureView` reuse first.

**Implementation approach:**

1. **Store pre-created views on `BlurredTextureEntry`.**
   - Add `texture_view: wgpu::TextureView` to `BlurredTextureEntry`.
   - Create it once when the texture is created/updated, and reuse it every frame in the composite pass.

2. **Kawase pass view reuse (cheap fix).**
   - `PooledTexture` in `texture_pool.rs:13` already stores both a `Texture` and a `TextureView`.
   - In `FilterPipeline::apply_kawase_frosted_glass()`, change all call sites that currently do `half_tex.create_view(&Default::default())` to use `half_tex.view()` instead.
   - This applies to both the `external_tex_set` branch (`filter_pipeline.rs:1088-1096`, which currently calls `.texture()`) and the `internal_pool` branch (`filter_pipeline.rs:1099-1103`).

**Validation:**
- Reduce `create_view` call counts to 0 per blur entry per frame in the composite steady state, and ~0 per Kawase pass for intermediate textures.

---

### 3.3 Replace Full-Screen Children Pass with Local Rendering (Task C)

**Current behavior:**
- Pass 3 creates a `w x h` "Children Texture" and renders all deferred children of all blur nodes into that texture (`lib.rs:1668-1684`).
- This is a second full-screen `render_to_texture` call.

**Desired behavior:**
- Render each blur node's children into a texture sized to that node's blur-need bounds, not the full viewport.

**Prerequisite:**
- The current deferred-child rendering (`render_deferred_child` at `lib.rs:2488`) only accepts a `parent_pos` and recursively accumulates coordinates via Taffy layouts. There is no existing "subtree bounds + local remapping" primitive. This must be introduced first.

**Implementation approach:**

1. **Add shared subtree bounds logic.**
   - Create a new helper `compute_subtree_bounds(id, state, parent_pos) -> Rect` that walks a node's descendants and accumulates their axis-aligned bounding boxes (using the same Taffy-layout-based coordinates that `render_deferred_child` uses).
   - Ensure padding logic matches the blur pass so children aren't clipped.

2. **Compute per-node children bounds.**
   - During `render_node_recursive_with_transform()`, when blur is applied, call `compute_subtree_bounds` on each deferred child.
   - Pad the union of those bounds by a small margin (e.g., 1–2 px for anti-aliasing, plus any node-specific effects such as shadows or borders). **Do not use `blur_radius * 2.5` here** — the deferred children texture holds foreground content, not the blurred background, so blur bleed is irrelevant.
   - Store `children_bounds: (f32, f32, f32, f32)` in `BlurredTextureEntry`.

3. **Create appropriately sized textures in Pass 3.**
   - Replace the single global `w x h` "Children Texture" with one texture per blur entry at `children_bounds` size.
   - Introduce a variant of `render_deferred_child` (or add an `origin_offset` parameter) that translates rendering by `-bounds.x, -bounds.y` so the children render into the local texture origin.

4. **Adjust composite pass.**
   - When compositing the children overlay in the final blit, use the local texture and draw it at the correct screen position (`bounds.x`, `bounds.y`).

**Risks & mitigations:**
- **Risk:** Multiple small textures increase descriptor/bind-group overhead.
  - *Mitigation:* The total pixel count will still be far lower than one fullscreen texture per blur frame; the GPU is usually fill-rate bound here.
- **Risk:** Children extending beyond the padded bounds get clipped.
  - *Mitigation:* Ensure padding covers only subtree content outsets (anti-aliasing, shadows, borders); log a warning if children overflow and fall back to a larger bounds.

---

### 3.4 Incremental Scene / Layout + Raster Cache Integration (Task D)

**Current behavior:**
- `render_node_recursive_with_transform()` walks the entire node tree every frame.
- Text editors are re-fetched and re-measured unconditionally.
- `compute_layout_with_measure()` is called with the full viewport size every frame.
- `Scene` is created fresh (`Scene::new()`) and fully repopulated.

**Desired behavior:**
- Only dirty subtrees trigger `Scene` rebuild.
- Static subtrees (no layout/text/style changes for N frames) are baked to textures via the existing `RasterCache`.
- `compute_layout_with_measure()` is skipped when no layout-affecting changes have occurred.

**Constraints discovered in code review:**
1. `DirtyTracker` (`dyxel_shared::DirtyTracker`) exists but is **not yet wired into the render backend's frame lifecycle** — there is no `clear()` call after rendering, and the backend does not currently consume it.
2. Text measurement itself can trigger `g.mark_dirty(id)` (`lib.rs:1110`). If we naively skip text remeasurement when `dirty_tracker.has_dirty() == false`, we miss cases where the editor's internal state changed without an explicit external dirty mark.
3. `RasterCache` stores `TextureId`, not `wgpu::TextureView` or a Vello `Image`. Drawing a cached subtree back into a parent `Scene` requires a new resource bridge.

Because of these constraints, Task D is split into **two independent phases**:

#### Phase D1: Layout Dirtiness Gating

Goal: establish a safe lifecycle for dirty tracking so we can skip redundant layout passes.

1. **Unify dirty tracking into a single source of truth.**
   - Today there are two parallel dirty systems:
     - `SharedState::mark_dirty()` (`state.rs:556`) drives Taffy dirty propagation.
     - `DirtyTracker` (`protocol.rs:330`) is a field-level bitmap meant for render gating.
   - **Prerequisite:** Before gating any pass on `DirtyTracker`, define which system is authoritative. The recommended approach is to make `SharedState::mark_dirty()` also call `dirty_tracker.mark_dirty(id, fields)` (or replace the ad-hoc Taffy-only dirty flag with `DirtyTracker`).
   - After each frame's render completes, call `dirty_tracker.clear()` so the next frame starts clean.
   - If the two systems diverge (e.g., Taffy is dirty but `DirtyTracker` is not), the layout gate will incorrectly skip `compute_layout_with_measure()` and produce stale frames.

2. **Make text remeasurement conditional on *either* dirty marks *or* editor staleness.**
   - Before the text-measure loop (`lib.rs:1090`), check if any text node's `editor.revision()` (or an equivalent editor-staleness signal) differs from the last measured revision.
   - **Prerequisite:** `dyxel_editor::Editor` currently does not expose a stable revision/generation counter. If none exists, add one (e.g., increment on every `set_text`, `set_style`, or internal layout invalidation) before implementing this gate.
   - If no dirty marks exist *and* no editor revisions changed, skip the entire text remeasurement pass.
   - If an editor revision changed but no dirty mark exists, remeasure that specific node and run `compute_layout_with_measure()` for it.

3. **Gate `compute_layout_with_measure()`.**
   - Only run when:
     - viewport size changed, OR
     - `dirty_tracker.has_dirty()` is true, OR
     - at least one text editor revision changed.

#### Phase D2: Subtree Raster Baking

Goal: integrate `RasterCache` so stable subtrees can be drawn from cached textures instead of rebuilt into the `Scene`.

1. **Resource bridge: `TextureId` -> Vello image draw.**
   - `RasterCache` returns a `TextureId` (an index into `GpuTexturePool`).
   - Add a method to `GpuTexturePool` (or the renderer) that resolves a `TextureId` to a `&wgpu::TextureView`.
   - Because Vello's `Scene` API accepts peniko `Image` objects, we need to decide how to inject a cached GPU texture:
     - **Option A (recommended):** Draw the cached texture as a post-Scene blit, just like blurred textures are composited today. Keep `render_node_recursive_with_transform` returning a list of "cached overlays" (node id + transform + bounds) and draw them after `render_to_texture`.
     - **Option B (future):** Extend the Vello `Scene` to support external image references. This is a larger upstream-vello dependency and out of scope for the immediate fix.
   - For the initial implementation, use **Option A**: when `RasterCache` hits, skip the subtree in `Scene` building and instead append a `CachedDraw` command to a per-frame list. During the final blit/composite pass, resolve each `CachedDraw`'s `TextureId` to a `TextureView` and draw a full-screen quad with it (same mechanism as blur composite).
   - **Layering constraint:** `CachedDraw` must be emitted at the exact same z-order position where the subtree would have appeared in the original `Scene`. Because the current pipeline is (1) main scene → scene texture, (2) blur sampling + composite, (3) deferred children overlay, a cached subtree that sits *under* a blur node must be drawn **before** the blur composite, while a cached subtree that sits *above* a blur node (or is a deferred child of one) must be drawn **after** the blur composite. For the first implementation, restrict cached draws to nodes that are **fully outside any blur subtree and do not have any blur-affected descendants**, and draw them as a single post-scene layer before blur compositing begins.
   - **Eligibility rule:** During `render_node_recursive_with_transform()`, explicitly check whether the current node or any of its ancestors/descendants participate in blur/deferred-children/composite. Do **not** rely solely on `node.has_blur == false`; a node may itself be plain while an ancestor or descendant is blurred.

2. **Integrate `RasterCache` into `render_node_recursive_with_transform()`.**
   - Add `raster_cache: &mut RasterCache` and `cached_draws: &mut Vec<CachedDraw>` parameters.
   - At the start of recursion for each node, check `cache.get_cached_texture(node_id)`.
   - If hit, push a `CachedDraw` and **skip the subtree entirely**.
   - If miss, proceed with normal scene building but call `cache.track_node(node_id, estimated_path_count)`.

3. **Frame-end stability update.**
   - After scene building, call `cache.next_frame()` and `cache.update_stability(dirty_tracker)`.
   - For each node returned as `ReadyToBake`, render its subtree to a pooled texture and call `cache.bake_node(id, texture_id, size)`.

4. **Path-count estimation.**
   - For simple rects: 1 path.
   - For rounded rects: 1 path.
   - For text: use editor's line count * 2 as a proxy.
   - For containers: sum of children until we exceed `min_path_count`.

**Risks & mitigations:**
- **Risk:** Over-aggressive caching causes stale frames when animations run.
  - *Mitigation:* `mark_dirty()` is already called on style/position/content changes; `RasterCache::update_stability()` resets baked nodes when dirty.
- **Risk:** Texture memory explosion from baking large containers.
  - *Mitigation:* `RasterCache` already has `memory_budget_mb` and LRU eviction via `check_memory_pressure()`.
- **Risk:** A cached node whose ancestor or descendant participates in blur/deferred-children/composite breaks z-order.
  - *Mitigation:* In the first implementation, restrict baking to nodes that are **fully outside any blur subtree** and **do not have any blur-affected descendants**. This means no node on the blur/deferred-children path can be baked.

---

## 4. Testing Strategy

1. **Unit tests for `render_with_blur` texture creation (Task A):**
   - Call `render_with_blur` twice with identical parameters; assert `create_texture` count is 1 (mock or instrumentation).

2. **View reuse test (Task B):**
   - Run a frame with a blur node; capture the number of `create_view` calls in the composite and Kawase paths. Assert it is <= 1 on the second identical frame.

3. **Children bounds test (Task C):**
   - Create a blur node with children at known positions; assert `compute_subtree_bounds` matches expected padded rectangle.
   - Visual regression: compare Pass 3 texture dimensions before/after the change.

4. **Raster cache integration test (Task D2):**
   - Render a static subtree for 30+ frames; assert `RasterCache` transitions to `Baked` and subsequent frames return the cached texture.
   - Mark a baked node dirty; assert it returns to `Tracking`.

5. **Layout gating test (Task D1):**
   - Render two frames with no changes; assert `compute_layout_with_measure` is skipped on the second frame.
   - Change a text editor's content without an explicit dirty mark; assert the node is still remeasured and relayout occurs.

6. **Visual regression / end-to-end:**
   - Run the sample app with blur cards; capture screenshots before and after optimizations to verify no visual changes.
   - Use `debug_frames_enabled()` output (PNG dumps) to inspect Pass 3 texture dimensions.

---

## 5. Success Metrics

| Metric | Baseline | Target |
|---|---|---|
| `Scene` rebuild CPU time (no dirty nodes) | ~2-5 ms | <0.1 ms |
| `device.create_texture` calls per frame per stable blur node | 1 | 0 |
| Pass 3 children texture pixel count | `w * h` (fullscreen) | `sum(entry_children_bounds_area)` |
| `create_view` calls per blur entry per frame (composite, steady state) | 1 | 0 |
| Kawase pass `create_view` calls per blur entry per frame | ~8-10 | 0 |
| `compute_layout_with_measure` calls per frame (static content) | 1 | 0 |

---

## 6. Implementation Order

1. **Task A:** Fix invalid `create_texture` in `render_with_blur` — smallest change, immediate GPU churn reduction.
2. **Task B:** Reuse blur composite / Kawase TextureViews — medium change, high GPU churn reduction. *Bind-group dynamic-offset reuse is deferred to a follow-up because it requires pipeline-layout changes.*
3. **Task C:** Replace full-screen children pass with local rendering — larger change, high bandwidth reduction. *Requires subtree-bounds helper first.*
4. **Task D1:** Wire up layout dirtiness gating — architectural prerequisite for D2.
5. **Task D2:** Subtree raster baking via `RasterCache` — biggest long-term CPU win. *Requires D1 and the resource bridge design.*
