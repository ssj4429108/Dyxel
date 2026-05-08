# Current `dyxel-render-vello` Architecture

_Last verified: 2026-05-07._

This document describes the architecture that is present in the code today. It is intentionally shorter than the historical design notes and focuses on the current runtime/backend split, frame lifecycle, platform differences, and known cleanup boundaries.

## Crate role

`dyxel-render-vello` is the Vello + wgpu rendering implementation for the renderer-agnostic APIs in `dyxel-render-api`.

The current code contains both:

- a drawing backend (`backend::VelloDrawingBackend`) used by the newer double-layer runtime path; and
- `VelloBackend`, which still owns most rendering pass orchestration and backs
  both the new drawing backend and the legacy direct `RenderBackend`
  compatibility module (`legacy_backend.rs`).

`VelloDrawingBackend` is intentionally thin: it validates that a frame came from `WgpuRuntime`, then delegates actual scene rendering to `VelloBackend`.

## High-level frame lifecycle

```text
platform / host
  -> WgpuRuntime::begin_frame(surface_id)
       - chooses device / queue / surface
       - may acquire a surface texture immediately
       - may allocate an offscreen frame target first on macOS / Android experiment paths
       - returns WgpuFrameContext
  -> VelloDrawingBackend::render(frame, package)
       - downcasts WgpuFrameContext
       - calls VelloBackend::render_to_view or render_with_surface_texture
       - stores last_submission_index for later GPU-ready waits
  -> WgpuRuntime::end_frame(frame)
       - presents the direct surface texture, or
       - late-acquires a surface texture and blits the offscreen frame, or
       - uses Android native/AHB experimental presenter when enabled
```

The backend render path does not own surface lifecycle. It renders into the view supplied by the runtime. Surface creation, resize, suspend/resume, frame acquisition, and presentation belong to `runtime.rs` / `frame_context.rs`.

## Runtime and frame context responsibilities

### `runtime.rs`

`WgpuRuntime` owns:

- `wgpu::Instance` creation;
- adapter/device/queue selection;
- `wgpu::Surface` creation and configuration;
- surface resize/suspend/resume;
- per-frame surface acquisition;
- late blit pipelines for offscreen-first presentation;
- Android present-mode and native-presenter experiment switches.

Native and wasm behavior is intentionally split with `cfg` gates. macOS/Android-only offscreen frame-ring state is compiled only on those targets. Android-only presenter state is compiled only on Android.

### `frame_context.rs`

`WgpuFrameContext` is the concrete `BackendFrameContext` for Vello/wgpu. It carries:

- `surface_texture` when rendering directly to a surface;
- `offscreen_texture` and `view` when rendering offscreen first;
- cloned `Device` / `Queue` handles;
- `format`, `width`, `height`, timing fields;
- `last_submission_index` for native GPU-ready waits;
- Android-only detached presenter state.

`BackendFrameContext` has a smaller wasm trait surface than native. Current implementation gates native-only methods (`supports_detached_present`, `present_detached`, `wait_until_gpu_ready`) with `#[cfg(not(target_arch = "wasm32"))]`.

## Backend render pass responsibilities

`VelloBackend` is still the top-level pass coordinator, but its fields are now
grouped into explicit owner structs in `state.rs`:

- `RendererState` — Vello renderer, pipeline cache, cold-start/loading state,
  cache metadata/deferred-init storage, memory tier initialization, AA
  selection, scoped pipeline-cache borrowing, and main scene-to-texture render
  helper.
- `BlitState` — blit shader/pipelines, sampler, backend triple buffer, and
  fullscreen blit draw/bind-group helpers. It also owns initialization-time
  blit resource storage, legacy surface-pipeline creation/storage, and the
  current triple-buffer slot accessor used by the frame coordinator.
- `BlurState` — filter pipeline, blur pipelines/buffers, blur entry lifetime,
  atlas/backdrop textures, blur texture pool.
- `RasterCacheState` — backend GPU raster-cache lookup, traversal-facing cached
  draw lookup/emission adapter, bake-plan execution shell, and GPU texture pool.
- `ShadowCacheState` — shadow cache entries/stats, renderer-id invalidation
  marker, and shadow drawing adapter.
- `TextCacheState` — glyph-run cache/stats and prepared-text drawing adapter.
- `DiagnosticsState` — perf monitor, overlay toggles, begin-frame accounting,
  frame timing, scheduler perf stats, and diagnostic logging accessors.

This is mostly a structural split: several low-coupling lifecycle helpers now
live on their state owners (`RendererState`, `BlitState`, `BlurState`,
`RasterCacheState`, `ShadowCacheState`, `TextCacheState`, `DiagnosticsState`).
`BlurState` also owns blur scene-entry lifetime management, pass-2 source
processing, pass-3 deferred-children rendering, pass-3.5 atlas packing, and
pass-4 blur composite command emission, while `VelloBackend` still coordinates
the overall sequence and owns the final render pass setup / cross-state
resource borrowing.
`BlitState` owns the actual fullscreen blit draw operation once the caller has
opened the render pass. The current main pass sequence is:

1. Per-frame housekeeping:
   - returned texture collection;
   - blur staging reset;
   - lazy/cold renderer initialization check.
2. Scene construction:
   - create a Vello `Scene` from the immutable `RenderPackage`;
   - traverse nodes and draw content through `scene_renderer.rs`;
   - collect raster-cache draw commands;
   - enqueue blur entries while traversing nodes, with blur-entry lifetime
     managed by `BlurState`.
3. Raster cache baking:
   - `RasterCacheState` executes bake/recycle plans supplied by core/runtime;
   - `VelloBackend` supplies the recursive scene-building callback used to
     populate each bake scene;
   - `RasterCacheState` stores GPU texture IDs in backend-owned lookup state.
4. Main scene render:
   - ask `RendererState` to render the scene into the backend triple-buffer
     texture.
5. Blur processing:
   - copy backdrop/source regions;
   - optionally use atlas-wide blur;
   - render deferred children for blur entries;
   - pack entries into the blur atlas;
   - composite blur over the final target.
6. Final blit:
   - open the final render pass on the runtime target view;
   - ask `BlitState` to blit the main triple-buffer texture;
   - composite cached draws and blur entries;
   - submit GPU work and return the submission index.

## Scene traversal

`scene_renderer.rs` owns the recursive node traversal and node drawing logic for
`VelloBackend`. It handles:

- raster-cache lookup during traversal through `RasterCacheLookup`;
- shadow draw delegation through `ShadowCacheState`;
- blur-entry creation while traversing blur nodes;
- text drawing through `TextCacheState` and rectangle drawing;
- recursive child traversal and layer push/pop behavior.

This is currently a file-level separation rather than a new state owner because
the traversal intentionally crosses shadow, text, blur, and raster-cache
concerns. Its internal `TraversalContext` groups the long-lived traversal inputs
and mutable outputs, while `NodeGeometry` / `LayerDecision` group per-node
position/size/transform and layer/blur/cache-subtree decisions. Leaf drawing
steps are split into small helpers inside the module (`draw_shadow_if_needed`,
`apply_blur_if_needed`, `draw_node_content`, cached-draw emission, child
recursion, and the layer push helper) to keep the recursive traversal body
focused on ordering. Shadow/text cache internals stay behind their state owner
methods rather than being accessed directly by traversal, and raster-cache
lookups are routed through the `RasterCacheLookup` adapter.

## Blur pipeline

Blur code is partially split into `blur/`, and blur resources are grouped under
`BlurState`. Scene-entry lifetime management, pass-2 blur source processing,
pass-3 deferred-children rendering, pass-3.5 atlas packing, and pass-4 blur
composite command emission now live on `BlurState`. Final pass-4 render-pass
setup is still coordinated by `VelloBackend`, so this is not yet a full
ownership split.

Important current modules:

- `blur/types.rs` — blur entry and texture data types.
- `blur/entry.rs` — entry construction and coordinate helpers.
- `blur/passes.rs` — pass-level blur helpers.
- `blur/atlas.rs` and `blur/atlas_pass.rs` — atlas layout and packing.
- `blur/children.rs` — `BlurState` pass-3 deferred child rendering for blur entries.
- `blur/composite.rs` — final blur compositing command emission.
- `blur/pipeline.rs` — blur pipeline/texture/buffer setup plus current
  `BlurState` pass-2 and atlas-packing orchestration.
- `filter_pipeline.rs` — Kawase/filter implementation used by blur passes.

Current blur state ownership is grouped under `VelloBackend::blur_state` fields
such as:

- `filter_pipeline`;
- `blur_composite_pipeline` and bind-group layouts;
- `blur_instanced_*` buffers/pipelines;
- `blurred_textures`;
- `backdrop_blur`;
- `blur_atlas` / `blur_source_atlas`;
- `texture_pool`.

A future behavior-preserving split should continue reducing the remaining
`VelloBackend` blur coordination surface while keeping the final wgpu render
pass lifetime owned by the top-level frame coordinator.

## Cache lifecycles

### Raster cache

The runtime/core decides what to bake via `RenderPackage::bake_plans`. The Vello backend executes bakes and maintains GPU-local lookup state:

- `cached_textures`: node ID to raster-cache texture ID;
- `gpu_texture_pool`: GPU texture allocation/reuse;
- `cache::CachedDraw`: draw commands for cached subtrees.

`RasterCacheState` owns recycle/acquire/record helpers, scoped cached-texture
lookup access (`RasterCacheLookup`), cached-draw emission, GPU texture-pool
installation/borrowing, and the bake-plan loop. `VelloBackend` still supplies
the recursive scene-rendering callback for each bake because node traversal uses
shadow/text/blur rendering helpers outside raster-cache ownership.

### Shadow cache

`shadow.rs` owns the shadow cache key/entry types and draw helper. Entries are keyed by quantized geometry/style and evicted when stale. The backend also caps per-frame shadow cache misses to avoid cold-frame GPU submit spikes.

### Text cache

`text.rs` caches prepared glyph runs by font, size, color, and glyph signature. It avoids rebuilding Vello glyph arrays for static text every frame and has simple size and LRU-style constraints.

### Pipeline/shader cache

Cold-start and staged initialization are split across:

- `cold_start.rs`;
- `minimal_shaders.rs`;
- `shader_cache.rs`;
- `staged_init.rs`;
- `staged_loader.rs`;
- `two_stage_init.rs`.

`RendererState` groups the renderer handle, pipeline cache, cache path, cache
stage, loading flag, loading thread handle, and small initialization helpers for
cache metadata, deferred init, scoped pipeline-cache borrowing, and memory
optimizer setup. `VelloBackend` still coordinates when renderer initialization
starts and when the resulting renderer is consumed.

## Platform-specific paths

### wasm32

- `SharedMutex<T>` is `RefCell<T>` through `dyxel-render-api`.
- Modules that call `.lock().unwrap()` on `SharedMutex` must import `LockExt` under `#[cfg(target_arch = "wasm32")]`.
- Cross-platform call sites that should not know the native/wasm concrete type
  can use `dyxel_render_api::lock_shared` / `try_lock_shared`, which return
  the platform-neutral `SharedMutexGuard`.
- Native-only frame context methods are not part of the wasm `BackendFrameContext` trait surface.
- Android/macOS offscreen presenter state is not compiled for wasm.

### macOS

- `WgpuRuntime::begin_frame` can use offscreen-first rendering to avoid blocking on drawable acquisition in the backend path.
- `end_frame` late-acquires the surface texture and blits the offscreen frame.

### Android

Android currently has several experimental/diagnostic paths:

- present mode override (`DYXEL_ANDROID_PRESENT_MODE`);
- max frame latency override;
- optional opaque surface mode;
- optional full-frame offscreen path;
- optional offscreen copy present;
- optional surface-ready wait;
- Android native presenter / SurfaceControl / AHardwareBuffer probes;
- wgpu AHB texture/frame experiments.

These are intentionally cfg-gated and should remain off by default unless explicitly enabled by environment variable.

## Runtime configuration sources

Runtime behavior is currently controlled by environment variables read in several modules:

- `runtime.rs` for Android present/acquire/offscreen behavior;
- `frame_context.rs` for detached blit wait behavior;
- `android_native_presenter.rs` and `android_native_wgpu_ahb.rs` for native presenter probes;
- `debug_utils.rs` for debug output location.

This is functional but distributed. A future `RuntimeConfig` / `AndroidRuntimeConfig` / `DiagnosticsConfig` should centralize one-shot configuration reads. Keep dynamic reads only for switches that are intentionally runtime-toggled.

## Test-only experimental modules

The following earlier layer/offscreen experiments have been moved under an explicit
test-only module:

- `crates/dyxel-render-vello/src/experimental/composite_pipeline.rs`
- `crates/dyxel-render-vello/src/experimental/layer.rs`

`lib.rs` mounts `experimental` only behind `#[cfg(test)]`, so these files are not
part of the production backend path. Their unit tests and type checks still run
with `cargo test -p dyxel-render-vello --lib`, preventing silent bitrot while
making their experimental status explicit.

`offscreen_renderer.rs` was also in this category and has been removed as dead code, matching the historical offscreen-cleanup plan. Leaving tracked-but-uncompiled modules increases architecture ambiguity.

## Current known architecture debt

- `VelloBackend` now has explicit state-owner groups, but it remains a large
  pass coordinator rather than a thin dispatcher.
- Several files still implement `impl VelloBackend`; many functions should move
  behind the relevant state owner now that the fields are grouped.
- `scene_renderer.rs` still couples traversal to shadow/text/blur/raster-cache
  rendering details.
- `filter_pipeline.rs`, `runtime.rs`, `lib.rs`, and
  `android_native_presenter.rs` remain large.
- Android configuration reads are still spread across modules.
- `SharedMutex` works cross-platform, but many existing modules still call
  `.lock().unwrap()` and therefore must remember the wasm `LockExt` import.
  New cross-platform helpers should prefer `lock_shared` / `try_lock_shared`
  when the code does not need a concrete mutex type.

## Recommended next behavior-preserving steps

1. Move methods behind the new state owners without changing algorithms:
   - `BlurState` already owns scene-entry lifetime and
     process/deferred-children/pack/composite helpers; the remaining blur
     orchestration boundary is the top-level final render-pass lifetime and
     cross-state resource borrowing.
   - `BlitState` already owns pipeline setup, triple-buffer lifecycle, texture
     bind-group creation, and fullscreen draw helpers; the remaining boundary is
     the caller-owned render pass and target selection.
   - `RasterCacheState` owns lookup/emission helpers, GPU texture-pool access,
     and the bake-plan shell; the remaining raster-cache boundary is separating
     recursive bake-scene construction from the main scene renderer.
   - `scene_renderer.rs` now has internal `TraversalContext`, `NodeGeometry`,
     `LayerDecision`, and leaf drawing helpers; shadow/text cache access is
     routed through state-owner drawing methods, and raster-cache lookup uses
     `RasterCacheLookup`. Recent cleanup also put the blit, renderer
     initialization, and diagnostics paths behind state-owner accessors. The
     next safe step would be moving more cold-start orchestration behind
     `RendererState` helpers or splitting `runtime.rs`/Android presenter
     responsibilities.
2. Move Android native presenter into submodules by responsibility:
   - properties/env;
   - dynamically loaded Android symbols;
   - SurfaceControl;
   - AHardwareBuffer lifecycle;
   - Vulkan import;
   - sync fd;
   - probes;
   - presenter orchestration.
3. Centralize runtime configuration construction.
4. Keep the validation matrix green after each step:

```bash
cargo check --workspace
cargo check -p dyxel-web --target wasm32-unknown-unknown
cargo check -p dyxel-render-vello --target wasm32-unknown-unknown
cargo test -p dyxel-render-vello --lib
cargo fmt -- --check
git diff --check
```
