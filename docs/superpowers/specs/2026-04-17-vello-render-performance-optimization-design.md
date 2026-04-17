# Vello Render Performance Optimization Design

> **Goal:** Eliminate per-frame full-scene CPU rebuild and GPU resource churn in the Vello render backend, focusing on incremental layout/scene caching, blur texture lifecycle fixes, and bind-group reuse.

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
| 1 | Incremental Scene / Layout + Raster Cache Integration | `lib.rs:1051-1217`, `raster_cache.rs`, `layer.rs` |
| 2 | Fix Invalid `create_texture` in `render_with_blur` | `lib.rs:2280-2430` |
| 3 | Replace Full-Screen Children Pass with Local Rendering | `lib.rs:1634-1715` |
| 4 | Reuse Blur Composite / Kawase Bind Groups and Views | `lib.rs:1976-2019`, `filter_pipeline.rs:1033-1165` |

**Out of scope:**
- Removing the 33 ms logic-thread boundary wait (`bridge.rs:27/775`) — input-latency improvement, not directly FPS.
- Adding a "no-effects direct-to-surface" fast path — valuable but secondary to the P0 items above.
- Cleaning debug/info logs in blur paths — trivial, can be done opportunistically.

---

## 3. Design Details

### 3.1 Incremental Scene / Layout + Raster Cache Integration

**Current behavior:**
- `render_node_recursive_with_transform()` walks the entire node tree every frame.
- Text editors are re-fetched and re-measured unconditionally.
- `compute_layout_with_measure()` is called with the full viewport size every frame.
- `Scene` is created fresh (`Scene::new()`) and fully repopulated.

**Desired behavior:**
- Only dirty subtrees trigger `Scene` rebuild.
- Static subtrees (no layout/text/style changes for N frames) are baked to textures via the existing `RasterCache`.
- `compute_layout_with_measure()` is skipped when the dirty tracker reports no layout-affecting changes.

**Implementation approach:**

1. **Wire up `DirtyTracker` in the render path.**
   - `dyxel_shared::DirtyTracker` already exists and is used by `RasterCache`.
   - Before the expensive text/layout passes, check `dirty_tracker.has_dirty_nodes()`.
   - If no dirty nodes, skip text re-measurement and full Taffy re-layout.

2. **Conditional `compute_layout_with_measure()`.**
   - Only run when at least one text node has changed size, or when the viewport size has changed, or when `dirty_tracker` indicates style/layout changes.

3. **Integrate `RasterCache` into `render_node_recursive_with_transform()`.**
   - Add a `raster_cache: &mut RasterCache` parameter.
   - At the start of recursion for each node, check `cache.get_cached_texture(node_id)`.
   - If hit, draw the cached texture into the parent `Scene` using an image draw and **skip the subtree entirely**.
   - If miss, proceed with normal scene building but call `cache.track_node(node_id, estimated_path_count)`.
   - After a node has been stable for `STABLE_FRAME_THRESHOLD` frames, bake it:
     - Allocate a texture from `GpuTexturePool` at the node's screen size.
     - Render the node's subtree to the texture via `render_to_texture`.
     - Store the texture ID in `RasterCache`.

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

---

### 3.2 Fix Invalid `create_texture` in `render_with_blur`

**Current behavior:**
- `render_with_blur()` unconditionally calls `device.create_texture(&texture_desc)` at `lib.rs:2326`.
- If an entry already exists for this `view_id` and the size hasn't changed, the newly created texture is simply dropped (overwritten at `lib.rs:2410` or never used at `lib.rs:2415`).

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

### 3.3 Replace Full-Screen Children Pass with Local Rendering

**Current behavior:**
- Pass 3 creates a `w x h` "Children Texture" and renders all deferred children of all blur nodes into it (`lib.rs:1668-1684`).
- This is a second full-screen `render_to_texture` call.

**Desired behavior:**
- Render each blur node's children into a texture sized to that node's blur-need bounds, not the full viewport.

**Implementation approach:**

1. **Compute per-node children bounds.**
   - During `render_node_recursive_with_transform()`, when blur is applied, calculate the bounding box of all deferred children (using their already-computed Taffy layouts).
   - Pad by `blur_radius * 2.5` to account for blur bleed.
   - Store `children_bounds: (f32, f32, f32, f32)` in `BlurredTextureEntry`.

2. **Create appropriately sized textures in Pass 3.**
   - Instead of one global `w x h` texture, create one texture per blur entry at `children_bounds` size.
   - Translate `children_scene` rendering by `-bounds.x, -bounds.y` so the children render into the local texture origin.

3. **Adjust composite pass.**
   - When compositing the children overlay in the final blit, use the local texture and draw it at the correct screen position.

**Risks & mitigations:**
- **Risk:** Multiple small textures increase descriptor/bind-group overhead.
  - *Mitigation:* The total pixel count will still be far lower than one fullscreen texture per blur frame; the GPU is usually fill-rate bound here.
- **Risk:** Children extending beyond the padded bounds get clipped.
  - *Mitigation:* Ensure padding matches the blur padding used in Pass 2; log a warning if children overflow and fall back to a larger bounds.

---

### 3.4 Reuse Blur Composite / Kawase Bind Groups and Views

**Current behavior:**
- In the blur composite loop (`lib.rs:1976-2019`), each entry does:
  ```rust
  let texture_view = entry.texture.create_view(...);
  let bind_group = device.create_bind_group(...);
  ```
- In `filter_pipeline.rs:1033-1165`, each Kawase pass creates a new `BindGroup` and calls `create_view(&Default::default())` on intermediate pool textures every pass.

**Desired behavior:**
- Avoid per-frame `create_bind_group` and `create_view` for stable resources.

**Implementation approach:**

1. **Store pre-created views on `BlurredTextureEntry`.**
   - Add `texture_view: wgpu::TextureView` to `BlurredTextureEntry`.
   - Create it once when the texture is created/updated, and reuse it every frame.

2. **Add a bind-group cache to `BlurredTextureEntry` or the renderer.**
   - The composite bind group depends on:
     - The texture view (stable per entry)
     - The sampler (global, stable)
     - The uniform buffer offset (changes every frame because we use a ring-buffer staging scheme)
   - Because the uniform offset changes, we cannot reuse the exact same `BindGroup` object across frames.
   - **Alternative:** Instead of caching the bind group, cache the `TextureView`. The `create_bind_group` cost is much lower than `create_view`, but on Android drivers both can hitch.
   - **Better alternative:** Use a bind-group-layout with dynamic offsets for the uniform buffer. Then the bind group can be created once per entry (texture + sampler + buffer, no fixed offset) and `rp.set_bind_group(0, &bind_group, &[dynamic_offset])` is used at draw time.
   - *Check:* Does the current pipeline layout support dynamic offsets? If not, add a new variant or update the existing `BlurCompositeBindGroupLayout`.

3. **Kawase pass view reuse.**
   - In `FilterPipeline::apply_kawase_frosted_glass()`, the intermediate textures (`half`, `quarter`, `ping`, `pong`) are from a pool.
   - Store `TextureView`s inside `KawaseTexturePool` so `run_pass` can borrow them instead of calling `create_view(&Default::default())` every time.

**Validation:**
- Reduce `create_bind_group` and `create_view` call counts to ~1 per blur entry per frame (composite) and ~0 per Kawase pass for intermediate textures.

---

## 4. Testing Strategy

1. **Unit tests for `render_with_blur` texture creation:**
   - Call `render_with_blur` twice with identical parameters; assert `create_texture` count is 1 (mock or instrumentation).

2. **Raster cache integration test:**
   - Render a static subtree for 30+ frames; assert `RasterCache` transitions to `Baked` and subsequent frames return the cached texture.
   - Mark a baked node dirty; assert it returns to `Tracking`.

3. **Children bounds test:**
   - Create a blur node with children at known positions; assert `children_bounds` matches expected padded rectangle.

4. **Visual regression / end-to-end:**
   - Run the sample app with blur cards; capture screenshots before and after optimizations to verify no visual changes.
   - Use `debug_frames_enabled()` output (PNG dumps) to inspect Pass 3 texture dimensions.

---

## 5. Success Metrics

| Metric | Baseline | Target |
|---|---|---|
| `Scene` rebuild CPU time (no dirty nodes) | ~2-5 ms | <0.1 ms |
| `device.create_texture` calls per frame per stable blur node | 1 | 0 |
| Pass 3 children texture pixel count | `w * h` (fullscreen) | `sum(entry_children_bounds_area)` |
| `create_bind_group` calls per blur entry per frame | 1 | 0 (with dynamic offsets) or 1 (cached) |
| Kawase pass `create_view` calls per blur entry per frame | ~8-10 | 0 |

---

## 6. Implementation Order

1. **Task A:** Fix invalid `create_texture` in `render_with_blur` — smallest change, immediate GPU churn reduction.
2. **Task B:** Reuse blur composite / Kawase bind groups and views — medium change, high GPU churn reduction.
3. **Task C:** Replace full-screen children pass with local rendering — larger change, high bandwidth reduction.
4. **Task D:** Incremental scene/layout + raster cache integration — largest architectural change, biggest long-term CPU win.
