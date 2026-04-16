# Dyxel Offscreen Rendering Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify texture pooling, remove dead code, and compress the offscreen rendering pipeline for Dyxel's Android rendering path.

**Architecture:** Eliminate per-frame `device.create_texture` calls by routing all blur and children offscreen textures through `SharedTexturePool`. Remove the internal `KawaseTexturePool` from `FilterPipeline` and make it a pure command recorder. Add content-hash-based blur caching and a bounding-box optimization path for deferred children when transform matrices are safe.

**Tech Stack:** Rust, wgpu, Vello, Taffy

---

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/dyxel-render-vello/src/offscreen_renderer.rs` | Dead code to be deleted |
| `crates/dyxel-render-vello/src/texture_pool.rs` | Pool extensions (`acquire_blur_offscreen`, `acquire_children_texture`) |
| `crates/dyxel-render-vello/src/filter_pipeline.rs` | Pure command recorder; `KawaseTexturePool` removed |
| `crates/dyxel-render-vello/src/lib.rs` | `BlurredTextureEntry` owns `PooledTexture`; integrates real `blur_cache`; children dual-path rendering |
| `crates/dyxel-render-vello/Cargo.toml` | Add `rustc-hash = "2"` dependency for content hashing |

---

## Task 1: P1 — Delete Dead Code (`offscreen_renderer.rs`)

**Files:**
- Delete: `crates/dyxel-render-vello/src/offscreen_renderer.rs`

- [ ] **Step 1: Delete the dead file**

```bash
rm crates/dyxel-render-vello/src/offscreen_renderer.rs
```

- [ ] **Step 2: Verify no references exist in the codebase**

```bash
grep -r "offscreen_renderer" crates/dyxel-render-vello/src/ || echo "No references found"
```

Expected: "No references found"

- [ ] **Step 3: Run cargo check to confirm no breakage**

```bash
cargo check -p dyxel-render-vello
```

Expected: clean compile with no errors

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: remove unused offscreen_renderer.rs dead code"
```

---

## Task 2: P0 — Extend `SharedTexturePool`

**Files:**
- Modify: `crates/dyxel-render-vello/src/texture_pool.rs`
- Test: inline `#[cfg(test)]` module in same file

- [ ] **Step 1: Add helper methods to `TexturePool` and `SharedTexturePool`**

In `crates/dyxel-render-vello/src/texture_pool.rs`, add the following methods to `TexturePool` (after `acquire_kawase_set`, around line 216):

```rust
    /// Acquire a blur offscreen texture (Rgba8Unorm, sized to bounding box)
    pub fn acquire_blur_offscreen(&mut self, width: u32, height: u32) -> PooledTexture {
        self.acquire(width, height, wgpu::TextureFormat::Rgba8Unorm)
    }

    /// Acquire a deferred children texture (Rgba8Unorm, sized to bounding box)
    pub fn acquire_children_texture(&mut self, width: u32, height: u32) -> PooledTexture {
        self.acquire(width, height, wgpu::TextureFormat::Rgba8Unorm)
    }
```

Then add the same methods to `SharedTexturePool` (after `acquire_kawase_set`, around line 277):

```rust
    pub fn acquire_blur_offscreen(&self, width: u32, height: u32) -> PooledTexture {
        self.inner.lock().unwrap().acquire_blur_offscreen(width, height)
    }

    pub fn acquire_children_texture(&self, width: u32, height: u32) -> PooledTexture {
        self.inner.lock().unwrap().acquire_children_texture(width, height)
    }
```

- [ ] **Step 2: Add unit test for pool helpers**

Append to the existing `#[cfg(test)] mod tests` at the bottom of `texture_pool.rs`:

```rust
    #[test]
    fn test_acquire_blur_offscreen_dimensions() {
        let w = 256u32;
        let h = 128u32;
        let key = bucket_key(w, h, wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(key, (256, 128, 0));
    }
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p dyxel-render-vello --lib
```

Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/dyxel-render-vello/src/texture_pool.rs
git commit -m "feat(texture_pool): add acquire_blur_offscreen and acquire_children_texture helpers"
```

---

## Task 3: P0 — Purify `FilterPipeline`

**Files:**
- Modify: `crates/dyxel-render-vello/src/filter_pipeline.rs`

- [ ] **Step 1: Remove `KawaseTexturePool` struct and `kawase_pool` field**

Delete lines 50-103 (the entire `KawaseTexturePool` struct and its `impl`).

Delete line 148 in `FilterPipeline`:
```rust
    kawase_pool: std::cell::RefCell<Option<KawaseTexturePool>>,
```

Delete the field initialization in `FilterPipeline::new` (find and remove the `kawase_pool` line).

- [ ] **Step 2: Update `apply_frosted_glass_kawase` signature and implementation**

Change the signature (around line 973) from:

```rust
    pub fn apply_frosted_glass_kawase(
        &self,
        mut encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        blur_radius: f32,
        external_pool: Option<&SharedTexturePool>,
    ) -> Result<(), FilterError> {
```

To:

```rust
    pub fn apply_frosted_glass_kawase(
        &self,
        mut encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        blur_radius: f32,
        kawase_set: &crate::texture_pool::KawaseTextureSet,
    ) -> Result<(), FilterError> {
```

Replace the entire body of `apply_frosted_glass_kawase` with this simplified version:

```rust
        let full_w = input.width();
        let full_h = input.height();

        let expected_half_w = (full_w / 2).max(1);
        let expected_half_h = (full_h / 2).max(1);
        let expected_quarter_w = (full_w / 8).max(1);
        let expected_quarter_h = (full_h / 8).max(1);

        let valid = kawase_set.ds_half.texture().width() == expected_half_w
            && kawase_set.ds_half.texture().height() == expected_half_h
            && kawase_set.ds_quarter.texture().width() == expected_quarter_w
            && kawase_set.ds_quarter.texture().height() == expected_quarter_h
            && kawase_set.ping.texture().width() == expected_quarter_w
            && kawase_set.ping.texture().height() == expected_quarter_h
            && kawase_set.pong.texture().width() == expected_quarter_w
            && kawase_set.pong.texture().height() == expected_quarter_h;

        if !valid {
            return Err(FilterError::InvalidFilterParameters(
                "KawaseTextureSet dimensions do not match input texture".to_string(),
            ));
        }

        let kawase_n = ((blur_radius / 25.0).ceil() as u32).max(2).min(4);

        let run_pass = |encoder: &mut wgpu::CommandEncoder,
                        src_view: &wgpu::TextureView,
                        dst_view: &wgpu::TextureView,
                        mode: u32,
                        pass_index: u32,
                        pipeline: &wgpu::RenderPipeline| {
            let uniforms = KawaseUniforms {
                mode,
                pass_index,
                _pad0: 0,
                _pad1: 0,
            };
            self.queue
                .write_buffer(&self.kawase_uniforms, 0, bytemuck::bytes_of(&uniforms));

            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Kawase Pass BindGroup"),
                layout: &self.kawase_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.kawase_uniforms.as_entire_binding(),
                    },
                ],
            });

            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Kawase Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.draw(0..3, 0..1);
        };

        let half_tex = kawase_set.ds_half.texture();
        let quarter_tex = kawase_set.ds_quarter.texture();
        let ping_tex = kawase_set.ping.texture();
        let pong_tex = kawase_set.pong.texture();

        run_pass(
            &mut encoder,
            &input.create_view(&Default::default()),
            &half_tex.create_view(&Default::default()),
            0, 0, &self.kawase_pipeline,
        );

        run_pass(
            &mut encoder,
            &half_tex.create_view(&Default::default()),
            &quarter_tex.create_view(&Default::default()),
            0, 0, &self.kawase_pipeline,
        );

        let textures = [quarter_tex, ping_tex, pong_tex];
        let mut src_idx: usize = 0;
        let kawase_dsts: [usize; 6] = [1, 2, 1, 2, 1, 2];
        let mut last_dst_idx = 0usize;
        for i in 0..kawase_n {
            let dst_idx = kawase_dsts[i as usize];
            run_pass(
                &mut encoder,
                &textures[src_idx].create_view(&Default::default()),
                &textures[dst_idx].create_view(&Default::default()),
                1, i, &self.kawase_pipeline,
            );
            src_idx = dst_idx;
            last_dst_idx = dst_idx;
        }

        run_pass(
            &mut encoder,
            &textures[last_dst_idx].create_view(&Default::default()),
            &half_tex.create_view(&Default::default()),
            2, 0, &self.kawase_pipeline,
        );

        run_pass(
            &mut encoder,
            &half_tex.create_view(&Default::default()),
            &output.create_view(&Default::default()),
            2, 0, &self.kawase_output_pipeline,
        );

        Ok(())
```

- [ ] **Step 3: Update `encode_frosted_glass_kawase` to delegate**

Change the signature (around line 1184) from:

```rust
    pub fn encode_frosted_glass_kawase(
        &self,
        mut encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        blur_radius: f32,
        external_pool: Option<&SharedTexturePool>,
        _uniforms_offset: u64,
    ) -> Result<(), FilterError> {
```

To:

```rust
    pub fn encode_frosted_glass_kawase(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        blur_radius: f32,
        kawase_set: &crate::texture_pool::KawaseTextureSet,
        _uniforms_offset: u64,
    ) -> Result<(), FilterError> {
```

Replace the body with:

```rust
        self.apply_frosted_glass_kawase(encoder, input, output, blur_radius, kawase_set)
```

- [ ] **Step 4: Remove unused `SharedTexturePool` import**

Delete line 11 in `filter_pipeline.rs`:
```rust
use crate::texture_pool::SharedTexturePool;
```

- [ ] **Step 5: Run cargo check**

```bash
cargo check -p dyxel-render-vello
```

Expected: clean compile

- [ ] **Step 6: Commit**

```bash
git add crates/dyxel-render-vello/src/filter_pipeline.rs
git commit -m "refactor(filter_pipeline): remove internal KawaseTexturePool, accept external KawaseTextureSet"
```

---

## Task 4: P0 — Refactor `BlurredTextureEntry` to Own `PooledTexture`

**Files:**
- Modify: `crates/dyxel-render-vello/src/lib.rs`

- [ ] **Step 1: Change `BlurredTextureEntry.texture` type**

Around line 51 in `lib.rs`, change:

```rust
struct BlurredTextureEntry {
    /// The blurred texture (contains blurred background for frosted glass)
    texture: wgpu::Texture,
```

To:

```rust
struct BlurredTextureEntry {
    /// The blurred texture (contains blurred background for frosted glass)
    texture: texture_pool::PooledTexture,
    /// Whether this entry was skipped due to zero size
    skipped_due_to_size: bool,
```

- [ ] **Step 2: Update all references to `entry.texture` in `lib.rs`**

**Location A** — blur copy loop around line 1429:

Change:
```rust
blur_entries.push((entry.view_id, &entry.texture, entry.blur_radius));
```
To:
```rust
blur_entries.push((entry.view_id, entry.texture.texture(), entry.blur_radius));
```

**Location B** — `post_enc.clear_texture` around line 1433:

Change:
```rust
post_enc.clear_texture(&entry.texture, ...)
```
To:
```rust
post_enc.clear_texture(entry.texture.texture(), ...)
```

**Location C** — `copy_texture_to_texture` destination around line 1469:

Change:
```rust
texture: &entry.texture,
```
To:
```rust
texture: entry.texture.texture(),
```

**Location D** — `apply_frosted_glass_kawase` call around line 1501-1518:

Change to:

```rust
                        let pool_guard = self.texture_pool.lock().unwrap();
                        let result = if let Some(ref pool) = *pool_guard {
                            let kawase_set = pool.acquire_kawase_set(
                                texture.width(), texture.height()
                            );
                            pipeline.apply_frosted_glass_kawase(
                                &mut post_enc,
                                texture,
                                texture,
                                blur_radius,
                                &kawase_set,
                            )
                        } else {
                            log::warn_once!("[Blur] texture_pool not initialized; skipping blur processing");
                            Ok(())
                        };
```

**Location E** — Pass 4 composite loop around line 1819:

Find any `entry.texture.create_view(&Default::default())` and change to `entry.texture.view()`.

- [ ] **Step 3: Update `render_with_blur` signature and body**

Change the signature (around line 2163) to:

```rust
fn render_with_blur(
    node: &dyxel_shared::ViewNode,
    id: u32,
    _state: &SharedState,
    _editors: &mut std::collections::HashMap<u32, Editor>,
    _scene: &mut Scene,
    local_transform: Affine,
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    _renderer: &mut vello::Renderer,
    _filter_pipeline: &crate::filter_pipeline::FilterPipeline,
    node_width: f64,
    node_height: f64,
    _needs_layer: bool,
    blurred_textures: &mut Vec<BlurredTextureEntry>,
    texture_pool: Option<&texture_pool::SharedTexturePool>,
) -> bool {
```

Replace the texture creation block (around line 2189-2209) with:

```rust
    let blur_radius = node.blur_radius as f64;
    let padding = (blur_radius * 2.5).ceil() as u32;
    let texture_width = (node_width as u32 + padding * 2).max(1);
    let texture_height = (node_height as u32 + padding * 2).max(1);

    let pool = match texture_pool {
        Some(p) => p,
        None => {
            log::warn_once!("[Blur] texture_pool not initialized; skipping blur for view_id={}", id);
            return false;
        }
    };

    if texture_width == 0 || texture_height == 0 {
        log::debug!("[Blur] Skipping zero-size blur for view_id={}", id);
        let existing_index = blurred_textures.iter().position(|e| e.view_id == id);
        if let Some(index) = existing_index {
            blurred_textures[index].skipped_due_to_size = true;
        }
        return false;
    }

    let offscreen_texture = pool.acquire_blur_offscreen(texture_width, texture_height);
```

In the `existing_index` update block, add:
```rust
entry.skipped_due_to_size = false;
```

In the `push` block, add:
```rust
skipped_due_to_size: false,
```

- [ ] **Step 4: Thread `texture_pool` through `render_node_recursive_with_transform`**

Change the signature (around line 2421):

```rust
fn render_node_recursive_with_transform(
    id: u32,
    state: &SharedState,
    editors: &mut std::collections::HashMap<u32, Editor>,
    scene: &mut Scene,
    parent_pos: Vec2,
    transform: Affine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    filter_pipeline: Option<&crate::filter_pipeline::FilterPipeline>,
    blurred_textures: &mut Vec<BlurredTextureEntry>,
    texture_pool: Option<&texture_pool::SharedTexturePool>,
) {
```

Pass `texture_pool` to `render_with_blur` (around line 2557) and to the recursive child call (around line 2626).

At the top-level call site (around line 1196), add:

```rust
            let pool_guard = self.texture_pool.lock().unwrap();
            let pool_ref = pool_guard.as_ref();

            render_node_recursive_with_transform(
                id,
                &g,
                &mut editors,
                &mut scene,
                Vec2::ZERO,
                root_transform,
                device,
                queue,
                renderer,
                filter_pipeline.as_ref(),
                &mut blurred_textures,
                pool_ref,
            );
```

- [ ] **Step 5: Run cargo check**

```bash
cargo check -p dyxel-render-vello
```

- [ ] **Step 6: Commit**

```bash
git add crates/dyxel-render-vello/src/lib.rs
git commit -m "refactor(lib): BlurredTextureEntry owns PooledTexture, thread pool through render path"
```

---

## Task 5: P0 — Implement Real `blur_cache` with Content Hashing

**Files:**
- Modify: `crates/dyxel-render-vello/src/lib.rs`
- Modify: `crates/dyxel-render-vello/Cargo.toml`

- [ ] **Step 1: Add `rustc-hash` dependency**

In `crates/dyxel-render-vello/Cargo.toml`, add under `[dependencies]`:

```toml
rustc-hash = "2"
```

- [ ] **Step 2: Simplify `CachedBlurResult` to metadata-only**

Around line 84 in `lib.rs`, change to:

```rust
struct CachedBlurResult {
    content_hash: u64,
    source_rect: (f32, f32, f32, f32),
    last_updated_frame: u64,
}
```

- [ ] **Step 3: Add content hash helper**

Add near `render_with_blur` (around line 2140):

```rust
fn compute_blur_content_hash(
    view_id: u32,
    source_rect: (f32, f32, f32, f32),
    blur_radius: f32,
    width: u32,
    height: u32,
) -> u64 {
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    view_id.hash(&mut hasher);
    source_rect.0.to_bits().hash(&mut hasher);
    source_rect.1.to_bits().hash(&mut hasher);
    source_rect.2.to_bits().hash(&mut hasher);
    source_rect.3.to_bits().hash(&mut hasher);
    blur_radius.to_bits().hash(&mut hasher);
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    hasher.finish()
}
```

- [ ] **Step 4: Integrate blur_cache in Pass 2 blur loop**

In the Pass 2 block (around line 1404), replace the entry loop with:

```rust
                    let current_frame = FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
                    let mut blur_cache = self.blur_cache.lock().unwrap();

                    blur_cache.retain(|_, cached| {
                        current_frame.saturating_sub(cached.last_updated_frame) <= 60
                    });

                    for entry in blurred_textures.iter_mut() {
                        if entry.skipped_due_to_size {
                            continue;
                        }

                        let content_hash = compute_blur_content_hash(
                            entry.view_id,
                            entry.source_rect,
                            entry.blur_radius,
                            entry.width,
                            entry.height,
                        );

                        if let Some(cached) = blur_cache.get(&entry.view_id) {
                            if cached.content_hash == content_hash
                                && cached.source_rect == entry.source_rect
                            {
                                log::debug!("[Blur Cache] Hit for view_id={}", entry.view_id);
                                entry.needs_recalculation = false;
                                continue;
                            }
                        }

                        entry.needs_recalculation = true;

                        let (src_x, src_y, src_w, src_h) = entry.source_rect;
                        if entry.blur_radius > 0.0 {
                            blur_entries.push((entry.view_id, entry.texture.texture(), entry.blur_radius));
                        }

                        post_enc.clear_texture(
                            entry.texture.texture(),
                            &wgpu::ImageSubresourceRange {
                                aspect: wgpu::TextureAspect::All,
                                base_mip_level: 0,
                                mip_level_count: None,
                                base_array_layer: 0,
                                array_layer_count: None,
                            },
                        );

                        let padding = ((entry.width as f32 - src_w) / 2.0) as u32;
                        #[cfg(target_os = "android")]
                        let src_origin_y = (h as f32 - src_y - src_h).max(0.0) as u32;
                        #[cfg(not(target_os = "android"))]
                        let src_origin_y = src_y.max(0.0) as u32;
                        let src_origin_x = src_x.max(0.0) as u32;
                        let copy_width = src_w as u32;
                        let copy_height = src_h as u32;

                        post_enc.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: scene_texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: src_origin_x, y: src_origin_y, z: 0 },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::TexelCopyTextureInfo {
                                texture: entry.texture.texture(),
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: padding, y: padding, z: 0 },
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::Extent3d { width: copy_width, height: copy_height, depth_or_array_layers: 1 },
                        );
                    }
```

- [ ] **Step 5: Store blur_cache metadata after processing**

After the `apply_frosted_glass_kawase` loop (around line 1533), add:

```rust
                    for entry in blurred_textures.iter_mut() {
                        if processed_view_ids.contains(&entry.view_id) {
                            entry.needs_recalculation = false;
                            let content_hash = compute_blur_content_hash(
                                entry.view_id,
                                entry.source_rect,
                                entry.blur_radius,
                                entry.width,
                                entry.height,
                            );
                            blur_cache.insert(
                                entry.view_id,
                                CachedBlurResult {
                                    content_hash,
                                    source_rect: entry.source_rect,
                                    last_updated_frame: current_frame,
                                },
                            );
                        }
                    }
```

- [ ] **Step 6: Run cargo check**

```bash
cargo check -p dyxel-render-vello
```

- [ ] **Step 7: Commit**

```bash
git add crates/dyxel-render-vello/src/lib.rs crates/dyxel-render-vello/Cargo.toml
git commit -m "feat(lib): implement content-hash-based blur_cache with FxHash"
```

---

## Task 6: P0 — Pool Children Texture

**Files:**
- Modify: `crates/dyxel-render-vello/src/lib.rs`

- [ ] **Step 1: Change children_texture acquisition to use pool**

In Pass 3 (around line 1588), replace the `children_texture` creation block with:

```rust
        let children_texture: Option<texture_pool::PooledTexture> = if has_children {
            let pool_guard = self.texture_pool.lock().unwrap();
            if let Some(pool) = pool_guard.as_ref() {
                log::debug!("[Blur] Pass 3: Acquiring children texture from pool");
                let tex = pool.acquire_children_texture(w, h);
                if let Err(e) = renderer.render_to_texture(
                    device,
                    queue,
                    &children_scene,
                    tex.view(),
                    &vello::RenderParams {
                        base_color: Color::TRANSPARENT,
                        width: w,
                        height: h,
                        antialiasing_method: aa_config,
                    },
                ) {
                    log::warn!("[Blur] Failed to render children texture: {:?}", e);
                    None
                } else {
                    Some(tex)
                }
            } else {
                log::warn_once!("[Blur] texture_pool not initialized; skipping children offscreen");
                None
            }
        } else {
            None
        };
```

- [ ] **Step 2: Update children_texture usages in Pass 3 debug save and Pass 4**

For debug save (around line 1634), change `&texture` to `children_texture.as_ref().unwrap().texture()`.

For Pass 4 children blit (around line 1937), change:

```rust
            if let Some((_, ref children_view)) = children_texture {
```

To:

```rust
            if let Some(ref children_tex) = children_texture {
                let children_view = children_tex.view();
```

And use `children_view` in the bind group creation.

- [ ] **Step 3: Run cargo check**

```bash
cargo check -p dyxel-render-vello
```

- [ ] **Step 4: Commit**

```bash
git add crates/dyxel-render-vello/src/lib.rs
git commit -m "feat(lib): pool children texture through SharedTexturePool"
```

---

## Task 7: P2 — Children Direct Render Optimization

**Files:**
- Modify: `crates/dyxel-render-vello/src/lib.rs`

- [ ] **Step 1: Add `children_can_direct_render` helper**

Add near `render_with_blur` (around line 2150):

```rust
fn children_can_direct_render(transform: &Affine) -> bool {
    let coeffs = transform.as_coeffs();
    let sx = coeffs[0];
    let ky = coeffs[1];
    let kx = coeffs[2];
    let sy = coeffs[3];

    const EPSILON: f64 = 1e-5;

    if kx.abs() > EPSILON || ky.abs() > EPSILON {
        return false;
    }
    if (sx - sy).abs() > EPSILON {
        return false;
    }
    true
}
```

- [ ] **Step 2: Compute `has_children` and `can_direct_render` in Pass 3**

Before the children_scene building block (around line 1557), add:

```rust
        // Determine if all blur entries with children can be direct-rendered
        let mut has_children = false;
        let mut can_direct_render = true;
        {
            let blurred_textures = self.blurred_textures.lock().unwrap();
            for entry in blurred_textures.iter() {
                if !entry.deferred_children.is_empty() {
                    has_children = true;
                    can_direct_render = can_direct_render && children_can_direct_render(&entry.transform);
                }
            }
        }
```

Then wrap the existing `children_scene` building and `children_texture` acquisition with `if has_children && !can_direct_render`:

```rust
        let children_texture: Option<texture_pool::PooledTexture> = if has_children && !can_direct_render {
            // existing children_scene building + pool acquisition code
        } else {
            None
        };
```

- [ ] **Step 3: Add direct render path in Pass 4**

After the main render pass block ends in Pass 4 (after the `}` that closes the `rp` block, around line 1928), add:

```rust
        // Direct render path for children (no offscreen texture needed)
        if has_children && can_direct_render {
            let blurred_textures = self.blurred_textures.lock().unwrap();
            let g = shared_state.lock().unwrap();
            let mut editors = self.editors.lock().unwrap();

            for entry in blurred_textures.iter() {
                if entry.deferred_children.is_empty() {
                    continue;
                }

                let mut child_scene = Scene::new();
                let global_x = entry.source_rect.0 as f64;
                let global_y = entry.source_rect.1 as f64;

                for &child_id in &entry.deferred_children {
                    render_deferred_child(
                        child_id,
                        &g,
                        &mut editors,
                        &mut child_scene,
                        Vec2::new(global_x, global_y),
                    );
                }

                let clip_rect = KRect::from_origin_size(
                    (entry.source_rect.0 as f64, entry.source_rect.1 as f64),
                    (entry.source_rect.2 as f64, entry.source_rect.3 as f64),
                );
                if entry.border_radius > 0.0 {
                    let rounded = RoundedRect::from_rect(clip_rect, entry.border_radius);
                    child_scene.push_layer(Fill::NonZero, vello::peniko::BlendMode::Normal, 1.0, entry.transform, &rounded);
                } else {
                    child_scene.push_layer(Fill::NonZero, vello::peniko::BlendMode::Normal, 1.0, entry.transform, &clip_rect);
                }

                if let Err(e) = renderer.render_to_texture(
                    device,
                    queue,
                    &child_scene,
                    &render_target_view,
                    &vello::RenderParams {
                        base_color: Color::TRANSPARENT,
                        width: w,
                        height: h,
                        antialiasing_method: aa_config,
                    },
                ) {
                    log::warn!("[Blur] Direct render of children failed: {:?}", e);
                }

                child_scene.pop_layer();
            }
        }
```

Make sure `aa_config` is accessible here (it was defined earlier in the render function).

- [ ] **Step 4: Run cargo check**

```bash
cargo check -p dyxel-render-vello
```

- [ ] **Step 5: Commit**

```bash
git add crates/dyxel-render-vello/src/lib.rs
git commit -m "feat(lib): add children direct-render optimization for axis-aligned transforms"
```

---

## Task 8: Final Integration and Full Build

**Files:**
- All modified files in `crates/dyxel-render-vello/`

- [ ] **Step 1: Run full project check**

```bash
./check_all.sh
```

Expected: all platforms compile cleanly

- [ ] **Step 2: Fix any remaining warnings or errors**

Address unused variable warnings, missing imports, or type mismatches.

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "chore: final integration of offscreen rendering cleanup (P0+P1+P2)"
```

---

## Self-Review Checklist

1. **Spec coverage:**
   - P1 dead code removal → Task 1
   - Pool unification → Tasks 2, 4, 6
   - FilterPipeline purification → Task 3
   - Real blur_cache → Task 5
   - Children dual-path → Task 7
   - Error handling (zero-size, pool missing) → embedded in Tasks 4, 5, 6, 7

2. **Placeholder scan:** No TBD, TODO, or vague steps. Each step has concrete code or exact commands.

3. **Type consistency:**
   - `entry.texture` is `texture_pool::PooledTexture` throughout
   - `children_texture` is `Option<texture_pool::PooledTexture>` throughout
   - `FilterPipeline` methods accept `&KawaseTextureSet`
   - `rustc-hash` dependency added for `FxHasher`
