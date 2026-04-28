// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_perf::{PerfConfig, PerformanceDiagnostics, PerformanceMonitor, SharedPerfMonitor};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use dyxel_render_api::{
    BackendConfig, DeviceHandle, LifecycleEvent, QueueHandle, RenderBackend, RenderBackendExt,
    RenderContext, RenderResult, SharedMutex, SurfaceHandle, SurfaceState,
    SurfaceTargetHandle, VelloBackendExt,
};
use kurbo::{Affine, Vec2};
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use vello::wgpu;
use vello::{
    Renderer, RendererOptions, Scene,
    peniko::Color,
};

// Two-stage init is implemented inline with cache header markers

#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_arch = "wasm32")]
pub mod web;

pub mod backend;
pub mod factory;
pub mod filter_pipeline;
pub mod frame_context;
pub mod minimal_shaders;
pub mod runtime;
pub mod scene_adapter;
pub mod shader_cache;
pub mod staged_init;
pub mod staged_loader;
pub mod texture_pool;
pub mod two_stage_init;

/// Vello render backend implementation
///
/// This is the concrete implementation of RenderBackend using Vello + wgpu
// Type aliases for shared data used in async context
type AsyncShared<T> = std::sync::Arc<std::sync::Mutex<T>>;

/// Granular dirty classification for a blur entry.
///
/// Replaces the binary `needs_recalculation` flag so that Pass 2/3/4 can
/// selectively skip work: an opacity-only change doesn't need a Kawase
/// re-blur, and a blur-radius change doesn't need a scene re-copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlurDirtyKind {
    /// No changes since last frame — skip entirely.
    Clean,
    /// The scene content behind the blur node changed — full re-copy + re-blur.
    BackgroundChanged,
    /// Blur parameters changed (radius, style, size, source_rect) — re-blur
    /// on the existing texture (no re-copy needed).
    BlurParamsChanged,
    /// Only opacity or overlay_color changed — update composite uniform only.
    OverlayOnlyChanged,
    /// Deferred children changed — re-render Pass 3 only.
    ChildrenChanged,
}

#[derive(Debug, Default)]
struct BlurDirtyStats {
    clean: usize,
    background: usize,
    params: usize,
    overlay: usize,
    children: usize,
    invalid: usize,
    pending: usize,
    skipped: usize,
    visible: usize,
    param_radius: usize,
    param_style: usize,
    param_src_x: usize,
    param_src_y: usize,
    param_src_w: usize,
    param_src_h: usize,
    bg_size: usize,
    children_list: usize,
    children_bounds: usize,
}

/// Entry for a blurred texture to be composited
#[derive(Debug)]
struct BlurredTextureEntry {
    /// The blurred texture (contains blurred background for frosted glass)
    texture: wgpu::Texture,
    /// Pre-created view of the blurred texture for composite reuse
    texture_view: wgpu::TextureView,
    /// Active blurred content width, including blur padding. This is the
    /// rectangle composited on screen and packed into the atlas.
    width: u32,
    /// Active blurred content height, including blur padding. This is the
    /// rectangle composited on screen and packed into the atlas.
    height: u32,
    /// Physical GPU texture width. Kept bucketed and >= width to avoid
    /// reallocating/invalidating blur textures when animated layout jitters
    /// around nearby sizes.
    allocated_width: u32,
    /// Physical GPU texture height. Kept bucketed and >= height.
    allocated_height: u32,
    /// Position to draw at (with padding offset already applied)
    transform: Affine,
    /// Opacity of the blurred content
    opacity: f32,
    /// View color to overlay (for frosted glass effect)
    overlay_color: vello::peniko::Color,
    /// Border radius
    border_radius: f64,
    /// Source rectangle in scene texture (for two-pass rendering)
    source_rect: (f32, f32, f32, f32), // (x, y, width, height) in scene coordinates
    /// Deferred children to render on top of blurred background
    deferred_children: Vec<u32>,
    /// Bounding box of deferred children in screen coordinates (x, y, width, height)
    children_bounds: (f32, f32, f32, f32),
    /// Cached children texture for local rendering (Pass 3)
    children_texture: Option<wgpu::Texture>,
    /// Cached view of the children texture
    children_texture_view: Option<wgpu::TextureView>,
    /// View ID for deferred rendering
    view_id: u32,
    /// Blur radius
    blur_radius: f32,
    /// Blur style: 0=Light, 1=Dark, 2=ExtraLight, 3=Prominent
    blur_style: u8,
    /// Skip this entry when the generated blur texture is intentionally disabled.
    skipped_due_to_size: bool,
    /// Per-frame dirty classification — drives selective Pass 2/3/4 execution.
    dirty_kind: BlurDirtyKind,
    /// Previous-frame blur_radius for change detection.
    prev_blur_radius: f32,
    /// Previous-frame source_rect for change detection.
    prev_source_rect: (f32, f32, f32, f32),
    /// Previous-frame opacity for change detection.
    prev_opacity: f32,
    /// Previous-frame overlay_color (RGBA) for change detection.
    prev_overlay_color: [u8; 4],
    /// Bitmask describing why the entry was classified as BlurParamsChanged.
    param_dirty_bits: u32,
    /// Backdrop pyramid LOD level (0=full-res, 1=half, 2=quarter, 3=eighth).
    backdrop_lod: u8,
    /// Scene-build frame marker used to retain only entries touched this frame.
    last_seen_frame: u64,
    /// Whether `texture` currently contains a valid blurred backdrop.
    ///
    /// This lets us defer rebuilds under load without compositing
    /// uninitialized/transparent blur textures.
    blur_valid: bool,
    /// A valid-but-stale blur that still needs a background re-copy/reblur.
    /// This lets Android keep budget=1 without dropping the visible frosted
    /// effect to transparent while pending entries catch up.
    blur_rebuild_pending: bool,
    /// Last frame in which this entry's blur texture was rebuilt.
    last_blur_rebuild_frame: u64,
    /// Cached resources for legacy per-entry compositing.
    composite_uniform_buffer: Option<wgpu::Buffer>,
    composite_overlay_buffer: Option<wgpu::Buffer>,
    composite_bind_group: Option<wgpu::BindGroup>,
    composite_uses_backdrop: bool,
    last_composite_uniform_data: Option<[f32; 12]>,
    last_composite_overlay_data: Option<[f32; 12]>,
    /// Cached resources for children compositing. Children texture/view is
    /// stable while bounds are stable, so recreating this bind group every
    /// frame is pure CPU/driver churn.
    children_uniform_buffer: Option<wgpu::Buffer>,
    children_overlay_buffer: Option<wgpu::Buffer>,
    children_bind_group: Option<wgpu::BindGroup>,
    last_children_uniform_data: Option<[f32; 12]>,
    last_children_overlay_data: Option<[f32; 12]>,
    /// Persistent atlas placement for instanced composite. The blurred texture
    /// content is copied into the atlas only when rebuilt or placement changes.
    atlas_valid: bool,
    atlas_dirty: bool,
    atlas_x: u32,
    atlas_y: u32,
}

/// A cached subtree draw command for the post-scene blit pass.
#[derive(Debug)]
struct CachedDraw {
    /// The cached texture identifier
    texture_id: texture_pool::TextureId,
    /// Transform that positions the texture on screen
    transform: Affine,
    /// Width of the cached texture in pixels
    width: f32,
    /// Height of the cached texture in pixels
    height: f32,
}

/// Cached blur result for a view
///
/// This allows skipping blur calculation when the view hasn't moved
/// and the background hasn't changed significantly.
#[derive(Debug)]
struct CachedBlurResult {
    content_hash: u64,
    source_rect: (f32, f32, f32, f32),
    last_updated_frame: u64,
}

/// Reusable full-screen blurred backdrop texture.
///
/// This replaces the expensive legacy path where every blur node owns and
/// rebuilds a separate blurred offscreen texture. The current implementation
/// produces one full-frame blur and lets all blur quads sample it in Pass 4.
struct BackdropBlurTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

struct BlurAtlasTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurInstance {
    // Screen-space quad rect including blur padding: x, y, width, height.
    rect: [f32; 4],
    // Atlas-space rect containing the active blurred texture.
    source_rect: [f32; 4],
    // overlay rgba.
    color: [f32; 4],
    // border_radius, opacity, dark_mode, unused.
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurFrameUniform {
    viewport_size: [f32; 2],
    _pad: [f32; 2],
}

/// Key for caching pre-rendered shadow textures.
/// Shadows with identical geometry + style can share the same cached texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ShadowCacheKey {
    /// Node width in pixels
    width: u16,
    /// Node height in pixels
    height: u16,
    /// Border radius (quantized to 0.5px, stored as x2)
    border_radius: u16,
    /// Shadow blur radius (quantized to 0.5px, stored as x2)
    blur_radius: u16,
    /// Shadow color (RGBA)
    color: [u8; 4],
}

struct ShadowCacheEntry {
    /// Vello ImageData registered with the renderer.
    /// The renderer internally holds the wgpu::Texture through image_overrides.
    image_data: peniko::ImageData,
    /// Last frame this entry was used (for LRU eviction)
    last_used_frame: AtomicU64,
}

#[derive(Default, Debug)]
struct ShadowCacheStats {
    hits: u64,
    misses: u64,
    evictions: u64,
}

/// Key for caching pre-built glyph runs.
/// Avoids re-iterating and re-mapping glyphs every frame for static text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GlyphRunCacheKey {
    font_ptr: usize,
    font_size_quanted: u32,
    color: [u8; 4],
    glyph_signature: u64,
}

struct GlyphRunCacheEntry {
    glyphs: Vec<vello::Glyph>,
    last_used_frame: AtomicU64,
}

#[derive(Default, Debug)]
struct GlyphRunCacheStats {
    hits: u64,
    misses: u64,
    evictions: u64,
}

/// A single slot in the triple-buffer ring.
pub(crate) struct TripleBufferSlot {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

/// Triple-buffered offscreen textures.
///
/// We rotate through 3 independent textures so that the GPU can still be
/// reading from frame N (final blit / present) while the CPU records
/// frame N+1 into a different texture. This eliminates the resource
/// contention that manifests as occasional JANK in Immediate mode.
pub(crate) struct TripleBuffer {
    slots: [TripleBufferSlot; 3],
    current_index: usize,
    width: u32,
    height: u32,
}

impl TripleBuffer {
    /// Advance the ring index.
    fn advance(&mut self) {
        self.current_index = (self.current_index + 1) % 3;
    }

    /// Return the currently-active slot.
    fn current(&self) -> &TripleBufferSlot {
        &self.slots[self.current_index]
    }
}

/// Frame counter for cache invalidation
static FRAME_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Monotonic frame marker for blur-entry lifetime/reuse across scene builds.
static BLUR_SCENE_FRAME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Node counter for debugging black screen (limit nodes to find breaking point)
static NODE_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Number of blur entries that may be fully rebuilt per frame.
/// Acts as a soft budget; entries beyond this limit are deferred to the next frame.
#[cfg(target_os = "macos")]
const MAX_BLUR_REBUILDS_PER_FRAME: usize = 8;
#[cfg(not(any(target_os = "android", target_os = "macos")))]
const MAX_BLUR_REBUILDS_PER_FRAME: usize = 6;
/// Keep normal performance runs from being dominated by Android logcat/macOS
/// unified logging. Per-frame info logs are useful while diagnosing, but they
/// are expensive enough to show up as Scene/Jank noise.
const DIAG_LOG_EVERY_N_FRAMES: u64 = 60;
const BLUR_SOURCE_RECT_EPS_PX: f32 = 1.0;
const BLUR_SOURCE_POS_BUCKET_PX: f32 = 16.0;
const BLUR_SOURCE_SIZE_BUCKET_PX: f32 = 16.0;
#[cfg(target_os = "android")]
const MAX_BLUR_REBUILDS_PER_FRAME_AT_60HZ: usize = 1;
/// Experimental but correctness-preserving path: pack every visible blur
/// backdrop into fixed atlas slots, run one Kawase blur over the atlas, then
/// composite from the blurred atlas. Unlike the rejected full-frame backdrop
/// path, this keeps per-entry backdrop semantics and padding.
const USE_ATLAS_WIDE_BACKDROP_BLUR: bool = false;
const BLUR_ATLAS_LEGACY_GAP_PX: u32 = 2;
// The atlas-wide path blurs the whole atlas texture. Keep a transparent moat
// between fixed slots so Kawase samples at slot edges do not bleed neighboring
// blur cards into each other.
const BLUR_ATLAS_WIDE_GAP_PX: u32 = 32;
const BLUR_ATLAS_MAX_DIM_PX: u32 = 4096;
#[cfg(target_os = "android")]
const BLUR_ATLAS_WIDE_MAX_SLOTS: usize = 24;
#[cfg(not(target_os = "android"))]
const BLUR_ATLAS_WIDE_MAX_SLOTS: usize = 48;
#[cfg(target_os = "android")]
const BLUR_ATLAS_WIDE_MAX_AREA_PX: u64 = 2_500_000;
#[cfg(not(target_os = "android"))]
const BLUR_ATLAS_WIDE_MAX_AREA_PX: u64 = 5_000_000;
const PARAM_DIRTY_RADIUS: u32 = 1 << 0;
const PARAM_DIRTY_STYLE: u32 = 1 << 1;
const PARAM_DIRTY_SRC_X: u32 = 1 << 2;
const PARAM_DIRTY_SRC_Y: u32 = 1 << 3;
const PARAM_DIRTY_SRC_W: u32 = 1 << 4;
const PARAM_DIRTY_SRC_H: u32 = 1 << 5;
/// Experimental full-frame backdrop blur path.
///
/// Disabled: visual result does not match the legacy per-entry backdrop blur.
/// Keep correctness first; optimization must preserve this legacy visual model.
const USE_FULL_FRAME_BACKDROP_BLUR: bool = false;

struct BlurAtlasLayout {
    width: u32,
    height: u32,
    slot: u32,
    gap: u32,
    placements: Vec<(usize, u32, u32)>,
}

#[inline]
fn ceil_sqrt_u32(n: u32) -> u32 {
    if n <= 1 {
        return n.max(1);
    }
    let mut x = (n as f64).sqrt().ceil() as u32;
    while x.saturating_mul(x) < n {
        x += 1;
    }
    x
}

fn compute_blur_atlas_layout(
    entries: &[BlurredTextureEntry],
    viewport_w: u32,
    viewport_h: u32,
    gap: u32,
) -> Option<BlurAtlasLayout> {
    let mut candidates: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.skipped_due_to_size && blur_entry_visible(entry, viewport_w, viewport_h))
        .map(|(idx, _)| idx)
        .collect();
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by_key(|&idx| entries[idx].view_id);

    let max_entry_extent = candidates
        .iter()
        .map(|&idx| {
            let entry = &entries[idx];
            entry.width.max(entry.height).saturating_add(gap.saturating_mul(2))
        })
        .max()
        .unwrap_or(0);

    let slot = if max_entry_extent <= 256 {
        256
    } else if max_entry_extent <= 320 {
        320
    } else if max_entry_extent <= 384 {
        384
    } else {
        return None;
    };

    let count = candidates.len() as u32;
    let max_cols = (BLUR_ATLAS_MAX_DIM_PX / slot).max(1);
    let mut cols = ceil_sqrt_u32(count).min(max_cols).max(1);
    let mut rows = (count + cols - 1) / cols;
    if rows.saturating_mul(slot) > BLUR_ATLAS_MAX_DIM_PX {
        cols = max_cols;
        rows = (count + cols - 1) / cols;
    }
    if rows.saturating_mul(slot) > BLUR_ATLAS_MAX_DIM_PX {
        return None;
    }

    let width = cols.saturating_mul(slot);
    let height = rows.saturating_mul(slot);
    let mut placements = Vec::with_capacity(candidates.len());
    for (slot_index, idx) in candidates.into_iter().enumerate() {
        let entry = &entries[idx];
        if entry.width.saturating_add(gap.saturating_mul(2)) > slot
            || entry.height.saturating_add(gap.saturating_mul(2)) > slot
        {
            return None;
        }
        let col = (slot_index as u32) % cols;
        let row = (slot_index as u32) / cols;
        placements.push((idx, col * slot + gap, row * slot + gap));
    }

    Some(BlurAtlasLayout {
        width,
        height,
        slot,
        gap,
        placements,
    })
}

#[inline]
fn kawase_pass_class_for_radius(radius: f32) -> u32 {
    ((radius / 25.0).ceil() as u32).max(2).min(4)
}

#[inline]
fn blur_atlas_wide_layout_within_budget(layout: &BlurAtlasLayout) -> bool {
    layout.placements.len() <= BLUR_ATLAS_WIDE_MAX_SLOTS
        && (layout.width as u64) * (layout.height as u64) <= BLUR_ATLAS_WIDE_MAX_AREA_PX
}

#[inline]
fn blur_texture_alloc_extent_px(active_extent: u32) -> u32 {
    // Blur cards in the current workload fit the 256px atlas slot. Allocating
    // the backing texture in coarse buckets keeps the active draw rect exact
    // while avoiding GPU texture churn and visible invalidation when layout
    // animation jitters by a few pixels.
    if active_extent <= 128 {
        128
    } else if active_extent <= 192 {
        192
    } else if active_extent <= 256 {
        256
    } else if active_extent <= 320 {
        320
    } else if active_extent <= 384 {
        384
    } else {
        ((active_extent + 63) / 64) * 64
    }
}

#[inline]
fn quantize_blur_pos_px(v: f32) -> f32 {
    (v / BLUR_SOURCE_POS_BUCKET_PX).round() * BLUR_SOURCE_POS_BUCKET_PX
}

#[inline]
fn quantize_blur_size_px(v: f32) -> f32 {
    // Use ceil for sizes to avoid clipping the source/backdrop when a layout
    // dimension falls between buckets.
    (v / BLUR_SOURCE_SIZE_BUCKET_PX).ceil() * BLUR_SOURCE_SIZE_BUCKET_PX
}

#[inline]
fn blur_rect_changed(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    (a.0 - b.0).abs() >= BLUR_SOURCE_RECT_EPS_PX
        || (a.1 - b.1).abs() >= BLUR_SOURCE_RECT_EPS_PX
        || (a.2 - b.2).abs() >= BLUR_SOURCE_RECT_EPS_PX
        || (a.3 - b.3).abs() >= BLUR_SOURCE_RECT_EPS_PX
}

#[inline]
fn blur_entry_visible(entry: &BlurredTextureEntry, viewport_w: u32, viewport_h: u32) -> bool {
    let (x, y, w, h) = entry.source_rect;
    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    x < viewport_w as f32 && y < viewport_h as f32 && x + w > 0.0 && y + h > 0.0
}
/// Deprecated: per-entry `BlurDirtyKind` now drives update decisions.
/// Kept as a fallback budget gate for extreme blur storms.
#[allow(dead_code)]
const BLUR_UPDATE_INTERVAL: u64 = 2;
#[allow(dead_code)]
const USE_CACHED_BLUR: bool = true;

/// Downsample pyramid of the rendered scene, used for LOD-based blur sampling.
///
/// Level 0 (not stored) = full-res scene texture.
/// Level 1 = half-res, Level 2 = quarter-res, Level 3 = eighth-res.
struct BackdropPyramid {
    half: wgpu::Texture,
    quarter: wgpu::Texture,
    eighth: wgpu::Texture,
    half_view: wgpu::TextureView,
    quarter_view: wgpu::TextureView,
    eighth_view: wgpu::TextureView,
}

/// Vello render backend implementation
///
/// This is the concrete implementation of RenderBackend using Vello + wgpu
pub struct VelloBackend {
    pub renderer: AsyncShared<Option<Renderer>>,
    pub blit_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    pub sampler: SharedMutex<Option<wgpu::Sampler>>,
    pub blit_shader: SharedMutex<Option<wgpu::ShaderModule>>,
    pub blit_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    /// Format the blit pipeline was created for; used to detect surface format changes.
    blit_pipeline_format: SharedMutex<Option<wgpu::TextureFormat>>,
    /// Triple buffer for offscreen compositing (managed internally, not per-surface).
    triple_buffer: SharedMutex<Option<TripleBuffer>>,
    // Pipeline for rendering children texture with alpha blending
    children_blit_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    pub pipeline_cache: AsyncShared<Option<wgpu::PipelineCache>>,
    pub cache_path: AsyncShared<Option<String>>,
    pub cache_saved: AtomicBool,
    // Current cache stage: None = no cache, Some(1) = Stage 1, Some(2) = Stage 2
    cache_stage: AsyncShared<Option<u8>>,
    // Deferred initialization - store device info for lazy init
    init_device_info: SharedMutex<Option<(String, Option<wgpu::PipelineCache>, Option<u8>)>>,
    // Performance monitoring
    perf_monitor: SharedPerfMonitor,
    // Detailed diagnostics (optional, for profiling)
    #[allow(dead_code)]
    diagnostics: SharedMutex<Option<PerformanceDiagnostics>>,
    // Performance overlay disabled (was using Editor; TODO: reimplement with PreparedText)
    // Memory optimizer for tiered memory configuration
    memory_optimizer: SharedMutex<dyxel_perf::MemoryOptimizer>,
    // Async initialization state tracking
    is_loading: std::sync::Arc<std::sync::atomic::AtomicBool>,
    // Async loading thread handle (optional - for monitoring)
    #[allow(dead_code)]
    loading_handle: SharedMutex<Option<std::thread::JoinHandle<()>>>,
    // Filter pipeline for blur effects
    filter_pipeline: SharedMutex<Option<filter_pipeline::FilterPipeline>>,
    // Blur composite pipeline for drawing blurred textures
    blur_composite_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    blur_composite_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    blur_composite_uniforms: SharedMutex<Option<wgpu::Buffer>>,
    blur_composite_overlay_uniforms: SharedMutex<Option<wgpu::Buffer>>,
    // Instanced backdrop blur composite path: one bind group + one draw for all blur quads.
    blur_instanced_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    blur_instanced_pipeline_format: SharedMutex<Option<wgpu::TextureFormat>>,
    blur_instanced_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    // Cached bind group for the global backdrop texture + stable per-frame buffers.
    // Invalidated when the backdrop texture, pipeline layout, frame uniform, or instance buffer changes.
    blur_instanced_bind_group: SharedMutex<Option<wgpu::BindGroup>>,
    blur_instance_buffer: SharedMutex<Option<wgpu::Buffer>>,
    blur_instance_capacity: SharedMutex<usize>,
    blur_frame_uniform: SharedMutex<Option<wgpu::Buffer>>,
    // Staging buffer for zero-copy blur uniform updates
    blur_staging_buffer: SharedMutex<Option<wgpu::Buffer>>,
    blur_staging_alignment: SharedMutex<usize>,
    blur_staging_offset: std::sync::atomic::AtomicUsize,
    // Blurred textures to composite (cleared each frame)
    blurred_textures: SharedMutex<Vec<BlurredTextureEntry>>,
    // Backdrop pyramid for LOD-based blur sampling (generated each frame after Pass 1)
    backdrop_pyramid: SharedMutex<Option<BackdropPyramid>>,
    // Full-screen blurred backdrop used by the new backdrop-filter path.
    backdrop_blur: SharedMutex<Option<BackdropBlurTexture>>,
    // Atlas used to batch-composite legacy-correct per-entry blurred textures.
    blur_atlas: SharedMutex<Option<BlurAtlasTexture>>,
    // Raw backdrop atlas used by the atlas-wide blur path. Each visible blur
    // entry is copied into its fixed slot with the same padding as the legacy
    // per-entry texture, then `blur_atlas` receives the blurred result.
    blur_source_atlas: SharedMutex<Option<BlurAtlasTexture>>,
    blur_atlas_wide_active_last_frame: AtomicBool,
    // Texture pool for efficient blur texture reuse
    texture_pool: SharedMutex<Option<texture_pool::SharedTexturePool>>,
    // GPU-local cache storage: node_id -> texture_id lookup table.
    // Runtime decides which nodes to bake (via bake_plans in RenderPackage).
    // Backend only executes bakes and performs read-only lookups during render.
    cached_textures: SharedMutex<std::collections::HashMap<u32, dyxel_render_api::raster_cache::TextureId>>,
    // GPU texture pool for raster cache baking
    gpu_texture_pool: SharedMutex<Option<texture_pool::GpuTexturePool>>,
    // Cached blur results (view_id -> cached result)
    blur_cache: SharedMutex<std::collections::HashMap<u32, CachedBlurResult>>,
    // Cached shadow textures (geometry+style key -> pre-rendered shadow)
    shadow_cache: SharedMutex<std::collections::HashMap<ShadowCacheKey, ShadowCacheEntry>>,
    // Shadow cache statistics for DIAG logging
    shadow_cache_stats: SharedMutex<ShadowCacheStats>,
    // Per-frame cap on shadow cache misses to avoid GPU submit spikes
    shadow_cache_misses_this_frame: AtomicU64,
    // Monotonically-incremented ID to detect renderer replacement (Stage 1 -> Stage 2).
    // Shadow cache entries are tied to a specific renderer instance.
    renderer_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    // Last seen renderer_id; mismatch means renderer was replaced and cache must be cleared.
    shadow_cache_renderer_id: std::sync::atomic::AtomicU64,
    // Frame timing from pacer (for DIAG logging)
    pacer_wait_ms: SharedMutex<f64>,
    frame_interval_ms: SharedMutex<f64>,
    // Cached glyph runs (font+text signature -> pre-built vello glyphs)
    glyph_run_cache: SharedMutex<std::collections::HashMap<GlyphRunCacheKey, GlyphRunCacheEntry>>,
    // Glyph run cache statistics for DIAG logging
    glyph_run_cache_stats: SharedMutex<GlyphRunCacheStats>,
    // Frame performance stats from scheduler (for DIAG logging)
    frame_perf_stats: SharedMutex<dyxel_perf::FramePerformanceStats>,
}

const BLIT_SHADER_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blit.spv"));

/// Convert neutral `[u8; 4]` (sRGB, non-premultiplied) to `peniko::Color`.
#[inline]
fn neutral_to_peniko_color(c: [u8; 4]) -> peniko::Color {
    peniko::Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Apply opacity to a neutral [u8; 4] color by scaling only the alpha channel.
/// Colors are non-premultiplied sRGB; opacity affects the final alpha only.
fn apply_opacity_to_color(c: [u8; 4], opacity: f32) -> [u8; 4] {
    if opacity >= 1.0 {
        c
    } else {
        let alpha = opacity.clamp(0.0, 1.0);
        [c[0], c[1], c[2], (c[3] as f32 * alpha) as u8]
    }
}

impl VelloBackend {
    pub fn new() -> Self {
        Self::with_perf_config(PerfConfig::default())
    }

    pub fn with_perf_config(perf_config: PerfConfig) -> Self {
        // Initialize memory optimizer with tiered configuration
        let memory_optimizer = dyxel_perf::MemoryOptimizer::new();
        log::info!(
            "[Memory] VelloBackend: Device tier detected: {:?}",
            memory_optimizer.tier()
        );

        Self {
            renderer: AsyncShared::new(std::sync::Mutex::new(None)),
            blit_bind_group_layout: SharedMutex::new(None),
            sampler: SharedMutex::new(None),
            blit_shader: SharedMutex::new(None),
            blit_pipeline: SharedMutex::new(None),
            blit_pipeline_format: SharedMutex::new(None),
            triple_buffer: SharedMutex::new(None),
            children_blit_pipeline: SharedMutex::new(None),
            pipeline_cache: AsyncShared::new(std::sync::Mutex::new(None)),
            cache_path: AsyncShared::new(std::sync::Mutex::new(None)),
            cache_saved: AtomicBool::new(false),
            cache_stage: AsyncShared::new(std::sync::Mutex::new(None)),
            init_device_info: SharedMutex::new(None),
            perf_monitor: std::sync::Arc::new(std::sync::Mutex::new(PerformanceMonitor::new(
                perf_config,
            ))),
            diagnostics: SharedMutex::new(Some(PerformanceDiagnostics::new(120))),
            memory_optimizer: SharedMutex::new(memory_optimizer),
            is_loading: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            loading_handle: SharedMutex::new(None),
            filter_pipeline: SharedMutex::new(None),
            blur_composite_pipeline: SharedMutex::new(None),
            blur_composite_bind_group_layout: SharedMutex::new(None),
            blur_composite_uniforms: SharedMutex::new(None),
            blur_composite_overlay_uniforms: SharedMutex::new(None),
            blur_instanced_pipeline: SharedMutex::new(None),
            blur_instanced_pipeline_format: SharedMutex::new(None),
            blur_instanced_bind_group_layout: SharedMutex::new(None),
            blur_instanced_bind_group: SharedMutex::new(None),
            blur_instance_buffer: SharedMutex::new(None),
            blur_instance_capacity: SharedMutex::new(0),
            blur_frame_uniform: SharedMutex::new(None),
            blur_staging_buffer: SharedMutex::new(None),
            blur_staging_alignment: SharedMutex::new(256),
            blur_staging_offset: std::sync::atomic::AtomicUsize::new(0),
            blurred_textures: SharedMutex::new(Vec::new()),
            backdrop_pyramid: SharedMutex::new(None),
            backdrop_blur: SharedMutex::new(None),
            blur_atlas: SharedMutex::new(None),
            blur_source_atlas: SharedMutex::new(None),
            blur_atlas_wide_active_last_frame: AtomicBool::new(false),
            texture_pool: SharedMutex::new(None),
            cached_textures: SharedMutex::new(std::collections::HashMap::new()),
            gpu_texture_pool: SharedMutex::new(None),
            blur_cache: SharedMutex::new(std::collections::HashMap::new()),
            shadow_cache: SharedMutex::new(std::collections::HashMap::new()),
            shadow_cache_stats: SharedMutex::new(ShadowCacheStats::default()),
            shadow_cache_misses_this_frame: AtomicU64::new(0),
            renderer_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
            shadow_cache_renderer_id: std::sync::atomic::AtomicU64::new(0),
            glyph_run_cache: SharedMutex::new(std::collections::HashMap::new()),
            glyph_run_cache_stats: SharedMutex::new(GlyphRunCacheStats::default()),
            pacer_wait_ms: SharedMutex::new(0.0),
            frame_interval_ms: SharedMutex::new(0.0),
            frame_perf_stats: SharedMutex::new(dyxel_perf::FramePerformanceStats::default()),
        }
    }

    /// Save texture to PNG file for debugging
    #[cfg(not(target_arch = "wasm32"))]
    fn save_texture_to_png(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        path: &str,
    ) {
        // Debug save disabled
        let size = texture.size();
        let format = texture.format();

        // wgpu requires bytes_per_row to be a multiple of 256
        let bytes_per_pixel = match format {
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm => 4,
            _ => 4,
        };
        let bytes_per_row_unaligned = size.width * bytes_per_pixel;
        let bytes_per_row = ((bytes_per_row_unaligned + 255) / 256) * 256;
        let buffer_size = (bytes_per_row * size.height) as u64;


        // Create buffer to read back
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Readback Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // Copy texture to buffer
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        queue.submit(Some(encoder.finish()));

        // Map and save
        let buffer_slice = readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        // Poll device until mapping completes
        while rx.try_recv().is_err() {
            let _ = device.poll(wgpu::PollType::Poll);
        }

        {
            let data = buffer_slice.get_mapped_range();
            let rgba_data: &[u8] = &data;

            // Copy row by row to handle alignment
            let mut img_data = Vec::with_capacity((size.width * size.height * 3) as usize);
            for row in 0..size.height {
                let row_start = (row * bytes_per_row) as usize;
                for col in 0..size.width {
                    let pixel_offset = row_start + (col * bytes_per_pixel) as usize;
                    if pixel_offset + 2 < rgba_data.len() {
                        // Handle BGRA vs RGBA
                        if format == wgpu::TextureFormat::Bgra8Unorm {
                            img_data.push(rgba_data[pixel_offset + 2]); // R (from B)
                            img_data.push(rgba_data[pixel_offset + 1]); // G
                            img_data.push(rgba_data[pixel_offset]); // B (from R)
                        } else {
                            img_data.push(rgba_data[pixel_offset]); // R
                            img_data.push(rgba_data[pixel_offset + 1]); // G
                            img_data.push(rgba_data[pixel_offset + 2]); // B
                        }
                    }
                }
            }

            let img = image::RgbImage::from_raw(size.width, size.height, img_data);
            if let Some(img) = img {
                if let Err(e) = img.save(path) {
                    log::warn!("Failed to save debug image {}: {}", path, e);
                }
            } else {
                log::warn!("Failed to create image from raw data");
            }
        }
        readback_buffer.unmap();
    }

    /// Check if debug frame saving is enabled
    #[cfg(not(target_arch = "wasm32"))]
    fn debug_frames_enabled(&self) -> bool {
        false // Disabled: debug frame saving is off by default
    }

    /// Get debug output directory
    #[cfg(not(target_arch = "wasm32"))]
    fn debug_output_dir(&self) -> std::path::PathBuf {
        let dir = std::env::var("DYXEL_DEBUG_DIR").unwrap_or_else(|_| "debug_frames".to_string());
        let path = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&path).ok();
        path
    }

    /// Enable performance overlay
    pub fn enable_perf_overlay(&self) {
        self.perf_monitor.lock().unwrap().toggle_overlay();
    }

    /// Disable performance overlay
    pub fn disable_perf_overlay(&self) {
        let mut monitor = self.perf_monitor.lock().unwrap();
        if monitor.should_show_overlay() {
            monitor.toggle_overlay();
        }
    }

    /// Async renderer initialization - non-blocking, runs in background thread
    /// Two-stage loading: Stage 1 (fast), save cache, Stage 2 (complete), update cache
    fn ensure_renderer_initialized_async(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Fast path - already initialized
        if self.renderer.lock().unwrap().is_some() {
            return;
        }

        // Check if already loading
        if self.is_loading.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }

        // Try to acquire init info
        let init_info = self.init_device_info.lock().unwrap().take();
        if init_info.is_none() {
            return; // No init info available (should not happen)
        }

        let (_cache_path, pipeline_cache, cache_stage) = init_info.unwrap();

        // Defensive: if self.pipeline_cache was never set (e.g. init raced), populate it now.
        {
            let mut pc = self.pipeline_cache.lock().unwrap();
            if pc.is_none() && pipeline_cache.is_some() {
                log::warn!(
                    "[ColdStart] self.pipeline_cache was None in ensure_renderer_initialized_async; restoring from init_device_info"
                );
                *pc = pipeline_cache.clone();
            }
        }

        let memory_tier = self.memory_optimizer.lock().unwrap().tier();

        // Determine if we need full load based on cache stage
        // cache_stage: None = no cache, Some(1) = Stage 1 (area_only), Some(2) = Stage 2 (full)
        let needs_full_load = cache_stage != Some(2);
        let is_first_launch = cache_stage.is_none();

        log::info!(
            "[ColdStart] Cache stage: {:?}, needs_full_load: {}, is_first_launch: {}",
            cache_stage,
            needs_full_load,
            is_first_launch
        );

        // Set loading flag
        self.is_loading
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // Clone necessary data for the background thread
        let renderer_clone = self.renderer.clone();
        let renderer_id_clone = self.renderer_id.clone();
        let is_loading_clone = self.is_loading.clone();
        let device_clone = device.clone();
        let queue_clone = queue.clone();
        let perf_monitor_clone = self.perf_monitor.clone();
        let cache_saved_clone = std::sync::Arc::new(AtomicBool::new(false));
        let cache_saved_for_thread = cache_saved_clone.clone();
        let pipeline_cache_clone = self.pipeline_cache.clone();
        let cache_path_clone: AsyncShared<Option<String>> = self.cache_path.clone();
        let cache_stage_clone = self.cache_stage.clone();

        // Spawn background thread for heavy shader compilation
        let handle = std::thread::spawn(move || {
            let start = std::time::Instant::now();

            // Determine AA support based on stage and tier
            let (aa_support, _stage_label) = if needs_full_load {
                if is_first_launch {
                    // First launch: Use area_only for fast startup
                    log::info!("[Vello] First launch: Using area_only AA for fast startup");
                    (vello::AaSupport::area_only(), "Stage 1 (first launch)")
                } else {
                    // Have Stage 1 cache, upgrading to full
                    log::info!("[Vello] Upgrading: Loading full AA support");
                    (vello::AaSupport::all(), "Stage 2 (upgrade)")
                }
            } else {
                // Have full cache
                log::info!("[Vello] Full cache hit: Using full AA support");
                (vello::AaSupport::all(), "Full cache")
            };

            // Determine thread count based on tier
            let num_threads = match memory_tier {
                dyxel_perf::DeviceMemoryTier::LowEnd => Some(2),
                dyxel_perf::DeviceMemoryTier::MidRange => Some(4),
                dyxel_perf::DeviceMemoryTier::HighEnd => {
                    std::thread::available_parallelism().ok().map(|n| n.get())
                }
            };

            let options = RendererOptions {
                antialiasing_support: aa_support,
                pipeline_cache,
                num_init_threads: num_threads.and_then(|n| std::num::NonZeroUsize::new(n)),
                use_cpu: false,
            };

            // Stage 1: Create renderer with appropriate AA mode
            let renderer_result = Renderer::new(&device_clone, options);

            match renderer_result {
                Ok(mut renderer) => {
                    log::info!(
                        "[ColdStart] Renderer::new() completed in {:?}",
                        start.elapsed()
                    );

                    // Perform minimal warmup
                    let warmup_start = std::time::Instant::now();
                    let dummy_texture = device_clone.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Async Warmup Texture"),
                        size: wgpu::Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::STORAGE_BINDING,
                        view_formats: &[],
                    });
                    let dummy_view =
                        dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());
                    let scene = Scene::new();
                    let params = vello::RenderParams {
                        base_color: Color::TRANSPARENT,
                        width: 1,
                        height: 1,
                        antialiasing_method: vello::AaConfig::Area,
                    };
                    let _ = renderer.render_to_texture(
                        &device_clone,
                        &queue_clone,
                        &scene,
                        &dummy_view,
                        &params,
                    );
                    log::info!(
                        "[ColdStart] Warmup completed in {:?}",
                        warmup_start.elapsed()
                    );

                    // Store renderer
                    *renderer_clone.lock().unwrap() = Some(renderer);
                    renderer_id_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    // Save Stage 1 cache only if we needed full load (first launch or Stage 1 upgrade)
                    // If we already had Stage 2 cache (needs_full_load=false), no need to save
                    if needs_full_load {
                        log::info!("[ColdStart] Saving Stage 1 cache");

                        let cache_lock = pipeline_cache_clone.lock().unwrap();
                        let path_lock = cache_path_clone.lock().unwrap();
                        if let (Some(cache), Some(path)) = (&*cache_lock, &*path_lock) {
                            if let Some(data) = cache.get_data() {
                                // Add header to mark as Stage 1
                                let mut cache_with_header = Vec::with_capacity(data.len() + 1);
                                cache_with_header.push(1u8); // Stage 1 marker
                                cache_with_header.extend_from_slice(&data);

                                if std::fs::write(path, &cache_with_header).is_ok() {
                                    cache_saved_for_thread
                                        .store(true, std::sync::atomic::Ordering::SeqCst);
                                    *cache_stage_clone.lock().unwrap() = Some(1);
                                    log::info!(
                                        "[ColdStart] Stage 1 cache saved ({} bytes)",
                                        cache_with_header.len()
                                    );
                                } else {
                                    log::error!(
                                        "[ColdStart] Failed to write Stage 1 cache to {}",
                                        path
                                    );
                                }
                            } else {
                                log::warn!("[ColdStart] Stage 1 cache get_data() returned None");
                            }
                        } else {
                            log::warn!(
                                "[ColdStart] Cannot save Stage 1 cache: cache={}, path={}",
                                cache_lock.is_some(),
                                path_lock.is_some()
                            );
                        }
                        drop(cache_lock);
                        drop(path_lock);
                    }

                    // Stage 2: If this is Stage 1 (first launch with area_only), upgrade to full in background
                    if is_first_launch && memory_tier != dyxel_perf::DeviceMemoryTier::LowEnd {
                        log::info!(
                            "[ColdStart] Starting Stage 2: Upgrading to full AA support in background"
                        );

                        let stage2_start = std::time::Instant::now();
                        let full_options = RendererOptions {
                            antialiasing_support: vello::AaSupport::all(),
                            pipeline_cache: pipeline_cache_clone.lock().unwrap().clone(),
                            num_init_threads: num_threads
                                .and_then(|n| std::num::NonZeroUsize::new(n)),
                            use_cpu: false,
                        };

                        // Try to create full renderer (will reuse Stage 1 cache + compile remaining)
                        match Renderer::new(&device_clone, full_options) {
                            Ok(full_renderer) => {
                                log::info!(
                                    "[ColdStart] Stage 2 complete in {:?}",
                                    stage2_start.elapsed()
                                );

                                // Replace the Stage 1 renderer with full renderer
                                *renderer_clone.lock().unwrap() = Some(full_renderer);
                                renderer_id_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                                // Save Stage 2 cache

                                let cache_lock = pipeline_cache_clone.lock().unwrap();
                                let path_lock = cache_path_clone.lock().unwrap();
                                if let (Some(cache), Some(path)) = (&*cache_lock, &*path_lock) {
                                    if let Some(data) = cache.get_data() {
                                        let mut cache_with_header =
                                            Vec::with_capacity(data.len() + 1);
                                        cache_with_header.push(2u8); // Stage 2 marker (full)
                                        cache_with_header.extend_from_slice(&data);

                                        if std::fs::write(path, &cache_with_header).is_ok() {
                                            log::info!(
                                                "[ColdStart] Stage 2 cache saved ({} bytes)",
                                                cache_with_header.len()
                                            );
                                            // Update cache_stage to Stage 2
                                            *cache_stage_clone.lock().unwrap() = Some(2);
                                        } else {
                                            log::error!(
                                                "[ColdStart] Failed to write Stage 2 cache to {}",
                                                path
                                            );
                                        }
                                    } else {
                                        log::warn!(
                                            "[ColdStart] Stage 2 cache get_data() returned None"
                                        );
                                    }
                                } else {
                                    log::warn!(
                                        "[ColdStart] Cannot save Stage 2 cache: cache={}, path={}",
                                        cache_lock.is_some(),
                                        path_lock.is_some()
                                    );
                                }
                            }
                            Err(e) => {
                                log::warn!(
                                    "[ColdStart] Stage 2 failed: {}, keeping Stage 1 renderer",
                                    e
                                );
                            }
                        }
                    }

                    // Record startup performance (Stage 1 time)
                    perf_monitor_clone
                        .lock()
                        .unwrap()
                        .record_startup_time(start.elapsed());
                }
                Err(e) => {
                    log::error!("[ColdStart] Failed to create renderer: {}", e);
                }
            }

            is_loading_clone.store(false, std::sync::atomic::Ordering::SeqCst);
        });

        *self.loading_handle.lock().unwrap() = Some(handle);
    }

    /// Check if renderer is ready for rendering
    pub fn is_renderer_ready(&self) -> bool {
        self.renderer.lock().unwrap().is_some()
    }

    /// Check if renderer is currently loading
    pub fn is_renderer_loading(&self) -> bool {
        self.is_loading.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn save_cache(&self) {
        if self.cache_saved.load(std::sync::atomic::Ordering::SeqCst) {
            log::info!("[ColdStart] Cache already saved, skipping");
            return;
        }
        let cache_lock = self.pipeline_cache.lock().unwrap();
        let path_lock = self.cache_path.lock().unwrap();
        let stage_lock = self.cache_stage.lock().unwrap();
        if let (Some(cache), Some(path)) = (&*cache_lock, &*path_lock) {
            #[cfg(not(target_arch = "wasm32"))]
            {
                log::info!("[ColdStart] Saving pipeline cache to: {}", path);
                if let Some(data) = cache.get_data() {
                    log::info!("[ColdStart] Cache data size: {} bytes", data.len());

                    // Add stage header if we have a valid stage
                    let result = if let Some(stage) = *stage_lock {
                        if stage == 1 || stage == 2 {
                            let mut cache_with_header = Vec::with_capacity(data.len() + 1);
                            cache_with_header.push(stage);
                            cache_with_header.extend_from_slice(&data);
                            log::info!("[ColdStart] Saving with Stage {} header", stage);
                            std::fs::write(path, &cache_with_header)
                        } else {
                            std::fs::write(path, &data)
                        }
                    } else {
                        std::fs::write(path, &data)
                    };

                    if let Err(e) = result {
                        log::error!("[ColdStart] Failed to save pipeline cache: {}", e);
                    } else {
                        log::info!(
                            "[ColdStart] Pipeline cache saved successfully ({} bytes)",
                            data.len()
                        );
                        self.cache_saved
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                } else {
                    log::warn!("[ColdStart] Cache get_data() returned None");
                }
            }
            #[cfg(target_arch = "wasm32")]
            let _ = (cache, path);
        } else {
            let has_cache = cache_lock.is_some();
            let has_path = path_lock.is_some();
            if !has_cache && has_path {
                log::warn!(
                    "[ColdStart] Cannot save cache: pipeline_cache object is None (PIPELINE_CACHE may not be supported by the adapter). path={}",
                    has_path
                );
            } else {
                log::warn!(
                    "[ColdStart] Cannot save cache: cache={}, path={}",
                    has_cache,
                    has_path
                );
            }
        }
    }

    /// Prewarm pipelines: create all necessary pipelines in background to reduce first-render latency
    fn prewarm_pipelines(&self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        log::info!("VelloBackend: Prewarming pipelines...");
        let blit_shader = self.blit_shader.lock().unwrap();
        let blit_layout = self.blit_bind_group_layout.lock().unwrap();

        if let (Some(shader), Some(layout)) = (&*blit_shader, &*blit_layout) {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Blit Pipeline Layout Prewarm"),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Blit Pipeline Prewarm"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: self.pipeline_cache.lock().unwrap().as_ref(),
            });
        *self.blit_pipeline.lock().unwrap() = Some(pipeline);
        *self.blit_pipeline_format.lock().unwrap() = Some(format);
        self.ensure_blur_instanced_resources(device, format, 128);
        }
        log::info!("VelloBackend: Pipeline prewarming complete.");
    }

    /// Ensure the blit pipeline matches the target surface format, recreating if needed.
    fn ensure_blit_pipeline(&self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        let needs_create = {
            let format_guard = self.blit_pipeline_format.lock().unwrap();
            format_guard.map_or(true, |f| f != format)
        };
        if needs_create {
            self.prewarm_pipelines(device, format);
        }
    }

    #[allow(dead_code)]
    /// Initialize blur composite pipeline for drawing blurred textures
    fn init_blur_composite_pipeline(&self, device: &wgpu::Device) {
        // Default to Rgba8Unorm, will be recreated with correct format if needed
        self.create_blur_composite_pipeline(device, wgpu::TextureFormat::Rgba8Unorm);
    }

    /// Create blur composite pipeline with specific format
    fn create_blur_composite_pipeline(&self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        // Create bind group layout with uniform buffer for transform and overlay
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blur Composite Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create uniform buffer (3 rows of vec4 = 48 bytes)
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blur Composite Uniform Buffer"),
            size: 48, // 3 * 16 bytes (aligned vec4s)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create overlay uniform buffer (color + radius + size + source rect = 48 bytes)
        let overlay_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blur Overlay Uniform Buffer"),
            size: 48, // 3 * 16 bytes (aligned vec4s)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Load shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blur Composite Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blur_composite.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blur Composite Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blur Composite Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: format,
                    // Premultiplied alpha blending: shader outputs premultiplied colors
                    // src_factor=One because RGB is already multiplied by alpha
                    // This correctly composites frosted glass over the main scene
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        *self.blur_composite_pipeline.lock().unwrap() = Some(pipeline);
        *self.blur_composite_bind_group_layout.lock().unwrap() = Some(bind_group_layout);
        *self.blur_composite_uniforms.lock().unwrap() = Some(uniform_buffer);
        *self.blur_composite_overlay_uniforms.lock().unwrap() = Some(overlay_uniform_buffer);

        // Initialize 1MB staging buffer for zero-copy blur uniform updates
        let alignment = device.limits().min_uniform_buffer_offset_alignment as usize;
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blur Staging Buffer"),
            size: 1024 * 1024,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        *self.blur_staging_buffer.lock().unwrap() = Some(staging_buffer);
        *self.blur_staging_alignment.lock().unwrap() = alignment;

        log::debug!("[Blur] Composite pipeline initialized");
    }

    fn ensure_backdrop_blur_texture(&self, device: &wgpu::Device, width: u32, height: u32) {
        let mut backdrop = self.backdrop_blur.lock().unwrap();
        let needs_create = backdrop
            .as_ref()
            .map_or(true, |tex| tex.width != width || tex.height != height);

        if needs_create {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Backdrop Blur Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            *backdrop = Some(BackdropBlurTexture {
                texture,
                view,
                width,
                height,
            });
            *self.blur_instanced_bind_group.lock().unwrap() = None;
        }
    }

    fn ensure_blur_atlas_texture(&self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let mut atlas = self.blur_atlas.lock().unwrap();
        let needs_create = atlas
            .as_ref()
            .map_or(true, |tex| tex.width != width || tex.height != height);
        if needs_create {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Blur Legacy Atlas Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            *atlas = Some(BlurAtlasTexture {
                texture,
                view,
                width,
                height,
            });
            *self.blur_instanced_bind_group.lock().unwrap() = None;
        }
        needs_create
    }

    fn ensure_blur_source_atlas_texture(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> bool {
        let mut atlas = self.blur_source_atlas.lock().unwrap();
        let needs_create = atlas
            .as_ref()
            .map_or(true, |tex| tex.width != width || tex.height != height);
        if needs_create {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Blur Raw Source Atlas Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            *atlas = Some(BlurAtlasTexture {
                texture,
                view,
                width,
                height,
            });
        }
        needs_create
    }

    fn ensure_blur_instanced_resources(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        instance_count: usize,
    ) {
        let pipeline_needs_create = {
            let pipeline = self.blur_instanced_pipeline.lock().unwrap();
            let pipeline_format = self.blur_instanced_pipeline_format.lock().unwrap();
            pipeline.is_none() || pipeline_format.map_or(true, |f| f != format)
        };

        if pipeline_needs_create {
            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Blur Instanced Bind Group Layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blur Atlas Instanced Composite Shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("blur_atlas_instanced_composite.wgsl").into(),
                ),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Blur Instanced Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Blur Instanced Composite Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

            *self.blur_instanced_pipeline.lock().unwrap() = Some(pipeline);
            *self.blur_instanced_pipeline_format.lock().unwrap() = Some(format);
            *self.blur_instanced_bind_group_layout.lock().unwrap() = Some(bind_group_layout);
            *self.blur_instanced_bind_group.lock().unwrap() = None;
        }

        let frame_needs_create = self.blur_frame_uniform.lock().unwrap().is_none();
        if frame_needs_create {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Blur Frame Uniform"),
                size: std::mem::size_of::<BlurFrameUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            *self.blur_frame_uniform.lock().unwrap() = Some(buffer);
            *self.blur_instanced_bind_group.lock().unwrap() = None;
        }

        let required_capacity = instance_count.max(1).next_power_of_two();
        let mut capacity = self.blur_instance_capacity.lock().unwrap();
        if self.blur_instance_buffer.lock().unwrap().is_none() || *capacity < required_capacity {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Blur Instance Buffer"),
                size: (required_capacity * std::mem::size_of::<BlurInstance>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            *self.blur_instance_buffer.lock().unwrap() = Some(buffer);
            *self.blur_instanced_bind_group.lock().unwrap() = None;
            *capacity = required_capacity;
        }
    }

    /// Clear surface with a simple color (fallback when renderer is loading)
    fn clear_surface(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_view: &wgpu::TextureView,
    ) -> RenderResult {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Clear Surface (Async Loading)"),
        });

        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), // Clear to black
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        queue.submit(Some(encoder.finish()));
        // Present is handled by the caller (old render_package) or GraphicsRuntime::end_frame (new path)

        Ok(())
    }

    /// Generate a 3-level downsample pyramid from the full-res scene texture.
    ///
    /// Level 1 = half-res, Level 2 = quarter-res, Level 3 = eighth-res.
    /// Uses the existing Kawase downsample pipeline (mode=0).
    /// Pyramid textures are Rgba8Unorm with COPY_SRC for subsequent
    /// `copy_texture_to_texture` usage in Pass 2.
    fn generate_backdrop_pyramid(
        &self,
        device: &wgpu::Device,
        scene_texture: &wgpu::Texture,
        scene_w: u32,
        scene_h: u32,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<BackdropPyramid> {
        let filter_pipeline = self.filter_pipeline.lock().unwrap();
        let pipeline = filter_pipeline.as_ref()?;

        let half_w = (scene_w / 2).max(1);
        let half_h = (scene_h / 2).max(1);
        let quarter_w = (scene_w / 4).max(1);
        let quarter_h = (scene_h / 4).max(1);
        let eighth_w = (scene_w / 8).max(1);
        let eighth_h = (scene_h / 8).max(1);

        let make_tex = |w: u32, h: u32, label: &str| -> (wgpu::Texture, wgpu::TextureView) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            (tex, view)
        };

        let (half, half_view) = make_tex(half_w, half_h, "Backdrop L1 (half)");
        let (quarter, quarter_view) = make_tex(quarter_w, quarter_h, "Backdrop L2 (quarter)");
        let (eighth, eighth_view) = make_tex(eighth_w, eighth_h, "Backdrop L3 (eighth)");

        // scene → half
        pipeline.run_kawase_downsample(
            encoder,
            &scene_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            &half_view,
        );
        // half → quarter
        pipeline.run_kawase_downsample(encoder, &half_view, &quarter_view);
        // quarter → eighth
        pipeline.run_kawase_downsample(encoder, &quarter_view, &eighth_view);

        log::debug!(
            "[BackdropPyramid] Built L1={}x{} L2={}x{} L3={}x{}",
            half_w, half_h,
            quarter_w, quarter_h,
            eighth_w, eighth_h,
        );

        Some(BackdropPyramid {
            half,
            quarter,
            eighth,
            half_view,
            quarter_view,
            eighth_view,
        })
    }

    fn render_internal_impl(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_view: &wgpu::TextureView,
        surface_format: wgpu::TextureFormat,
        package: &dyxel_render_api::RenderPackage,
    ) -> RenderResult {
        // Derive render inputs from the immutable package (no runtime objects)
        let node_map: std::collections::HashMap<u32, &dyxel_render_api::SceneNode> =
            package.nodes.iter().map(|n| (n.id, n)).collect();
        let rid = package.root_id;
        let w = package.viewport.0;
        let h = package.viewport.1;
        let diag_seq = FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
        let diag_log_this_frame = diag_seq % DIAG_LOG_EVERY_N_FRAMES == 0;
        if diag_log_this_frame {
            log::info!("[DIAG] Package: nodes={} root_id={:?} viewport={}x{}", package.nodes.len(), rid, w, h);
            if let Some(first) = package.nodes.first() {
                log::info!("[DIAG] First node: id={} xy=({:.1},{:.1}) wh=({:.1},{:.1}) opacity={:.2} content={:?}", first.id, first.x, first.y, first.width, first.height, first.opacity, std::mem::discriminant(&first.content));
            }
        }

        // Backend-internal frame housekeeping (was prepare_internal)
        if let Some(ref pool) = *self.texture_pool.lock().unwrap() {
            pool.collect_returns();
        }
        self.blur_staging_offset
            .store(0, std::sync::atomic::Ordering::Relaxed);

        // Detailed frame timing for diagnostics
        let frame_start = std::time::Instant::now();
        let mut stage_timer = dyxel_perf::FrameTimer::new();

        // Async initialization: start background compilation without blocking
        self.ensure_renderer_initialized_async(device, queue);
        stage_timer.mark("init_check");

        // Check if renderer is ready
        let mut renderer_lock = self.renderer.lock().unwrap();
        let renderer = match renderer_lock.as_mut() {
            Some(r) => {
                if diag_log_this_frame {
                    log::info!("[DIAG] Renderer ready");
                }
                r
            }
            None => {
                // Renderer not ready yet - clear surface and return
                log::info!("[DIAG] Renderer not ready, clearing surface");
                // This keeps the main loop at 60fps while shader compiles in background
                drop(renderer_lock); // Release lock before calling clear_surface
                return self.clear_surface(device, queue, target_view);
            }
        };

        // Begin frame timing for performance monitoring
        {
            let monitor = self.perf_monitor.lock().unwrap();
            monitor.begin_frame();
        }
        stage_timer.mark("perf_start");

        if w == 0 || h == 0 {
            return Ok(());
        }

        // Reset per-frame shadow cache miss counter
        self.shadow_cache_misses_this_frame
            .store(0, std::sync::atomic::Ordering::Relaxed);

        // Shadow cache LRU eviction: remove entries unused for 300 frames (5s @ 60fps)
        {
            let current_frame = FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
            let mut cache = self.shadow_cache.lock().unwrap();
            let mut stats = self.shadow_cache_stats.lock().unwrap();
            let before = cache.len();
            let evicted: Vec<peniko::ImageData> = cache
                .extract_if(|_, entry| {
                    let last = entry.last_used_frame.load(std::sync::atomic::Ordering::Relaxed);
                    current_frame.saturating_sub(last) > 300
                })
                .map(|(_, entry)| entry.image_data)
                .collect();
            let after = cache.len();
            if before != after {
                stats.evictions += (before - after) as u64;
                log::debug!("[ShadowCache] Evicted {} entries ({} -> {})", before - after, before, after);
            }
            // Unregister evicted textures from renderer to prevent image_overrides bloat
            drop(cache);
            drop(stats);
            for image_data in evicted {
                renderer.unregister_texture(image_data);
            }
        }

        // Detect renderer replacement (e.g. Stage 1 -> Stage 2 cold-start upgrade).
        // When renderer is replaced, image_overrides are lost; stale cache entries
        // would trigger "invalid empty image" panic on draw_image.
        {
            let current_id = self
                .renderer_id
                .load(std::sync::atomic::Ordering::Relaxed);
            let last_id = self
                .shadow_cache_renderer_id
                .load(std::sync::atomic::Ordering::Relaxed);
            if last_id != 0 && last_id != current_id {
                log::warn!(
                    "[ShadowCache] Renderer replaced (id {} -> {}), clearing shadow cache",
                    last_id,
                    current_id
                );
                let mut cache = self.shadow_cache.lock().unwrap();
                for (_, entry) in cache.drain() {
                    renderer.unregister_texture(entry.image_data);
                }
            }
            self.shadow_cache_renderer_id
                .store(current_id, std::sync::atomic::Ordering::Relaxed);
        }

        // Glyph run cache LRU eviction: remove entries unused for 300 frames
        {
            let current_frame = FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
            let mut cache = self.glyph_run_cache.lock().unwrap();
            let mut stats = self.glyph_run_cache_stats.lock().unwrap();
            let before = cache.len();
            cache.retain(|_, entry| {
                let last = entry.last_used_frame.load(std::sync::atomic::Ordering::Relaxed);
                let keep = current_frame.saturating_sub(last) <= 300;
                if !keep {
                    stats.evictions += 1;
                }
                keep
            });
            let after = cache.len();
            if before != after {
                log::debug!("[GlyphCache] Evicted {} entries ({} -> {})", before - after, before, after);
            }
        }

        let mut scene = Scene::new();
        let mut cached_draws: Vec<CachedDraw> = Vec::new();

        if let Some(id) = rid {
            stage_timer.mark("state_lock");

            // DEBUG: Reset node counter for this frame
            NODE_COUNTER.store(0, std::sync::atomic::Ordering::SeqCst);

            // Apply platform correction at the root level
            let root_transform = platform_correction(h as f64);
            let blur_scene_frame =
                BLUR_SCENE_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;

            // Get filter pipeline for blur effects
            let filter_pipeline = self.filter_pipeline.lock().unwrap();
            let mut blurred_textures = self.blurred_textures.lock().unwrap();
            // Keep entries across frames so blur/children textures can be reused.
            // Stale entries are removed after the scene walk via last_seen_frame.

            self.render_node_recursive_with_transform(
                id,
                &node_map,
                &mut scene,
                Vec2::ZERO,
                root_transform,
                device,
                queue,
                renderer,
                filter_pipeline.as_ref(),
                &mut blurred_textures,
                &mut cached_draws,
                false,
                blur_scene_frame,
            );
            blurred_textures.retain(|entry| entry.last_seen_frame == blur_scene_frame);
            stage_timer.mark("scene_build");
        }

        // === Execute recycle plans produced by Runtime cache policy ===
        {
            let mut cached_textures_guard = self.cached_textures.lock().unwrap();
            let mut gpu_texture_pool = self.gpu_texture_pool.lock().unwrap();
            if let Some(pool) = gpu_texture_pool.as_mut() {
                for plan in &package.recycle_plans {
                    pool.release(texture_pool::TextureId(plan.texture_id.0));
                    cached_textures_guard.remove(&plan.node_id);
                }
            }
        }

        // === Execute bake plans produced by Runtime cache policy ===
        // LIMIT: process at most 2 bake plans per frame to prevent render time spikes.
        const MAX_BAKES_PER_FRAME: usize = 2;
        stage_timer.mark("bake_start");
        {
            let mut cached_textures_guard = self.cached_textures.lock().unwrap();
            let mut gpu_texture_pool = self.gpu_texture_pool.lock().unwrap();
            if let Some(pool) = gpu_texture_pool.as_mut() {
                for plan in package.bake_plans.iter().take(MAX_BAKES_PER_FRAME) {
                    let tex_w = plan.width;
                    let tex_h = plan.height;
                    if tex_w == 0 || tex_h == 0 {
                        continue;
                    }

                    let texture_id =
                        pool.acquire(tex_w, tex_h, wgpu::TextureFormat::Rgba8Unorm);
                    if let Some(ptex) = pool.get_texture(texture_id) {
                        let mut bake_scene = Scene::new();
                        let mut bake_blurred = Vec::new();
                        self.render_node_recursive_internal(
                            plan.node_id,
                            &node_map,
                            &mut bake_scene,
                            Vec2::ZERO,
                            Affine::IDENTITY,
                            device,
                            queue,
                            renderer,
                            None,
                            &mut bake_blurred,
                            &*cached_textures_guard,
                            &mut Vec::new(),
                            false,
                            0,
                        );
                        let _ = renderer.render_to_texture(
                            device,
                            queue,
                            &bake_scene,
                            ptex.view(),
                            &vello::RenderParams {
                                base_color: Color::TRANSPARENT,
                                width: tex_w,
                                height: tex_h,
                                antialiasing_method: vello::AaConfig::Area,
                            },
                        );
                        cached_textures_guard.insert(
                            plan.node_id,
                            dyxel_render_api::raster_cache::TextureId(texture_id.0),
                        );
                    }
                }
            }
        }
        stage_timer.mark("bake_done");

        // Triple-buffering: create / resize the ring when dimensions change.
        let mut triple_buffer = self.triple_buffer.lock().unwrap();
        let needs_recreate = triple_buffer
            .as_ref()
            .map_or(true, |tb| tb.width != w || tb.height != h);
        if needs_recreate {
            let layout = self
                .blit_bind_group_layout
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .clone();
            let sampler = self.sampler.lock().unwrap().as_ref().unwrap().clone();

            let make_slot = || {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Vello Offscreen Texture (TripleBuffer)"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Vello Blit Bind Group (TripleBuffer)"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });
                TripleBufferSlot {
                    texture,
                    view,
                    bind_group,
                }
            };

            let tb_new = TripleBuffer {
                slots: [make_slot(), make_slot(), make_slot()],
                current_index: 0,
                width: w,
                height: h,
            };

            // Cold-start fix: initialize newly-created GPU textures to transparent.
            // Without this, uninitialized texture memory may display as white/gray
            // during the first frame while shaders are still compiling.
            let mut init_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("TripleBuffer Init Clear"),
            });
            for slot in &tb_new.slots {
                init_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Init Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &slot.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            queue.submit([init_enc.finish()]);

            *triple_buffer = Some(tb_new);
        }

        let tb = triple_buffer.as_mut().unwrap();
        tb.advance();

        // Tier-based AA configuration: reduce quality for LowEnd to save memory
        let multiplier = self
            .memory_optimizer
            .lock()
            .unwrap()
            .vello_buffer_multiplier();
        let aa_config = if multiplier < 0.5 {
            vello::AaConfig::Area // LowEnd: use simpler AA
        } else {
            vello::AaConfig::Area // Default to Area for consistent performance
        };

        // Single render: main scene + overlay (if enabled) to offscreen texture
        log::debug!("[Blur] Rendering scene to texture {}x{}", w, h);
        let enc = scene.encoding();
        if diag_log_this_frame {
            log::info!("[DIAG] Scene encoding: empty={} n_paths={} n_clips={} n_open_clips={} path_tags={} draw_tags={}", enc.is_empty(), enc.n_paths, enc.n_clips, enc.n_open_clips, enc.path_tags.len(), enc.draw_tags.len());
        }

        renderer
            .render_to_texture(
                device,
                queue,
                &scene,
                &tb.current().view,
                &vello::RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: w,
                    height: h,
                    antialiasing_method: aa_config,
                },
            )
            .map_err(|e| anyhow::anyhow!("render_to_texture failed: {:?}", e))?;
        stage_timer.mark("gpu_render");

        // OPTIMIZATION: Removed blocking wait. GPU commands are naturally ordered by submission.
        // The copy operations in Pass 2 will execute after the scene render completes.
        // This allows CPU to continue preparing blur commands while GPU renders the scene.

        // Debug: Save scene texture after Pass 1
        #[cfg(not(target_arch = "wasm32"))]
        let _debug_enabled = self.debug_frames_enabled();
        #[cfg(not(target_arch = "wasm32"))]
        if _debug_enabled {
            let frame_num = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let debug_dir = self.debug_output_dir();
            {
                let scene_tex = &tb.current().texture;
                let path = debug_dir.join(format!("frame_{:06}_pass1_scene.png", frame_num % 1000));
                self.save_texture_to_png(device, queue, scene_tex, path.to_str().unwrap());

                // Debug: Sample pixels at blur card locations (expected to show purple background)
                log::debug!(
                    "[Debug] Sampling scene texture at blur card locations (expected purple bg)"
                );
            }
        }

        // === PASS 2: Process blur textures from scene ===
        // Early-out: skip all blur work when there are no blurred textures.
        let has_blur = !self.blurred_textures.lock().unwrap().is_empty();

        let mut post_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Frame Encoder"),
        });

        // Keep the old lower-res pyramid disabled. It was fast, but on real
        // devices it degraded into a tint-only fill. The full-frame path below
        // uses an actual Kawase blurred full-resolution scene texture.
        *self.backdrop_pyramid.lock().unwrap() = None;
        if !USE_FULL_FRAME_BACKDROP_BLUR {
            *self.backdrop_blur.lock().unwrap() = None;
        }
        // Do not clear `blur_instanced_bind_group` here. With the visually
        // correct legacy path it is the atlas bind group, and the atlas texture,
        // frame uniform, and instance buffer are persistent. Clearing it per
        // frame forces `device.create_bind_group` every frame and reintroduces
        // CPU/driver tail jitter. Resource creation paths below invalidate it
        // when the atlas/pipeline/buffers actually change.

        let mut atlas_wide_blur_valid_this_frame = false;
        let mut atlas_wide_source_copies_this_frame = 0usize;

        if has_blur {
            // Legacy/correctness path: every blur entry copies its own backdrop
            // region from the rendered scene texture and runs Kawase blur into
            // that local texture. This is more expensive, but restores the real
            // frosted blur effect instead of showing a flat color overlay.
            let current_frame = FRAME_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut blurred_textures = self.blurred_textures.lock().unwrap();
            let filter_pipeline = self.filter_pipeline.lock().unwrap();
            let scene_texture = &tb.current().texture;
            let dirty_count = blurred_textures
                .iter()
                .filter(|e| e.dirty_kind != BlurDirtyKind::Clean)
                .count();
            let mut dirty_stats = BlurDirtyStats::default();
            for entry in blurred_textures.iter() {
                if entry.skipped_due_to_size {
                    dirty_stats.skipped += 1;
                }
                if blur_entry_visible(entry, w, h) {
                    dirty_stats.visible += 1;
                }
                if !entry.blur_valid {
                    dirty_stats.invalid += 1;
                }
                if entry.blur_rebuild_pending {
                    dirty_stats.pending += 1;
                }
                if entry.dirty_kind == BlurDirtyKind::BlurParamsChanged {
                    if entry.param_dirty_bits & PARAM_DIRTY_RADIUS != 0 {
                        dirty_stats.param_radius += 1;
                    }
                    if entry.param_dirty_bits & PARAM_DIRTY_STYLE != 0 {
                        dirty_stats.param_style += 1;
                    }
                    if entry.param_dirty_bits & PARAM_DIRTY_SRC_X != 0 {
                        dirty_stats.param_src_x += 1;
                    }
                    if entry.param_dirty_bits & PARAM_DIRTY_SRC_Y != 0 {
                        dirty_stats.param_src_y += 1;
                    }
                    if entry.param_dirty_bits & PARAM_DIRTY_SRC_W != 0 {
                        dirty_stats.param_src_w += 1;
                    }
                    if entry.param_dirty_bits & PARAM_DIRTY_SRC_H != 0 {
                        dirty_stats.param_src_h += 1;
                    }
                }
                if entry.dirty_kind == BlurDirtyKind::BackgroundChanged {
                    dirty_stats.bg_size += 1;
                }
                if entry.dirty_kind == BlurDirtyKind::ChildrenChanged {
                    if entry.deferred_children.is_empty() {
                        dirty_stats.children_list += 1;
                    } else {
                        dirty_stats.children_bounds += 1;
                    }
                }
                match entry.dirty_kind {
                    BlurDirtyKind::Clean => dirty_stats.clean += 1,
                    BlurDirtyKind::BackgroundChanged => dirty_stats.background += 1,
                    BlurDirtyKind::BlurParamsChanged => dirty_stats.params += 1,
                    BlurDirtyKind::OverlayOnlyChanged => dirty_stats.overlay += 1,
                    BlurDirtyKind::ChildrenChanged => dirty_stats.children += 1,
                }
            }
            let max_radius = blurred_textures
                .iter()
                .filter(|e| !e.skipped_due_to_size)
                .map(|e| e.blur_radius)
                .fold(0.0f32, f32::max);
            if current_frame % DIAG_LOG_EVERY_N_FRAMES == 0 {
                log::info!(
                    "[BlurLegacy] Frame {} — {} entries, {} visible, {} dirty, max_radius={:.1}, stats clean={} bg={} params={} overlay={} children={} invalid={} pending={} skipped={} param_bits radius={} style={} x={} y={} w={} h={} bg_size={} child_list={} child_bounds={}",
                    current_frame,
                    blurred_textures.len(),
                    dirty_stats.visible,
                    dirty_count,
                    max_radius,
                    dirty_stats.clean,
                    dirty_stats.background,
                    dirty_stats.params,
                    dirty_stats.overlay,
                    dirty_stats.children,
                    dirty_stats.invalid,
                    dirty_stats.pending,
                    dirty_stats.skipped,
                    dirty_stats.param_radius,
                    dirty_stats.param_style,
                    dirty_stats.param_src_x,
                    dirty_stats.param_src_y,
                    dirty_stats.param_src_w,
                    dirty_stats.param_src_h,
                    dirty_stats.bg_size,
                    dirty_stats.children_list,
                    dirty_stats.children_bounds,
                );
            }

            if USE_FULL_FRAME_BACKDROP_BLUR {
                if let Some(pipeline) = filter_pipeline.as_ref() {
                    self.ensure_backdrop_blur_texture(device, w, h);
                    let backdrop_texture = {
                        let backdrop = self.backdrop_blur.lock().unwrap();
                        backdrop.as_ref().map(|b| b.texture.clone())
                    };
                    if let Some(backdrop_texture) = backdrop_texture {
                        post_enc.copy_texture_to_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: scene_texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::TexelCopyTextureInfo {
                                texture: &backdrop_texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            wgpu::Extent3d {
                                width: w,
                                height: h,
                                depth_or_array_layers: 1,
                            },
                        );
                        stage_timer.mark("blur_copy_submit");
                        let pool_guard = self.texture_pool.lock().unwrap();
                        if let Err(e) = pipeline.apply_frosted_glass_kawase(
                            &mut post_enc,
                            &backdrop_texture,
                            &backdrop_texture,
                            max_radius,
                            pool_guard.as_ref(),
                        ) {
                            log::warn!("[BlurBackdropFull] Kawase failed: {:?}", e);
                        }
                        for entry in blurred_textures.iter_mut() {
                            entry.blur_valid = true;
                            entry.blur_rebuild_pending = false;
                            entry.dirty_kind = BlurDirtyKind::Clean;
                        }
                        stage_timer.mark("blur_render_submit");
                    } else {
                        stage_timer.mark("blur_copy_submit");
                        stage_timer.mark("blur_render_submit");
                    }
                } else {
                    stage_timer.mark("blur_copy_submit");
                    stage_timer.mark("blur_render_submit");
                }
            } else if let Some(pipeline) = filter_pipeline.as_ref() {
                if USE_ATLAS_WIDE_BACKDROP_BLUR {
                    let layout = compute_blur_atlas_layout(
                        &blurred_textures,
                        w,
                        h,
                        BLUR_ATLAS_WIDE_GAP_PX,
                    );
                    let radius_class_reference = layout.as_ref().and_then(|layout| {
                        layout
                            .placements
                            .first()
                            .map(|(idx, _, _)| {
                                kawase_pass_class_for_radius(blurred_textures[*idx].blur_radius)
                            })
                    });
                    let radius_class_uniform = if let (Some(layout), Some(reference)) =
                        (layout.as_ref(), radius_class_reference)
                    {
                        layout.placements.iter().all(|(idx, _, _)| {
                            kawase_pass_class_for_radius(blurred_textures[*idx].blur_radius)
                                == reference
                        })
                    } else {
                        false
                    };

                    if let Some(layout) = layout {
                        let atlas_wide_within_budget =
                            blur_atlas_wide_layout_within_budget(&layout);
                        if layout.placements.len() >= 8
                            && radius_class_uniform
                            && atlas_wide_within_budget
                        {
                            self.ensure_blur_atlas_texture(device, layout.width, layout.height);
                            self.ensure_blur_source_atlas_texture(
                                device,
                                layout.width,
                                layout.height,
                            );
                            let source_atlas_texture = {
                                let guard = self.blur_source_atlas.lock().unwrap();
                                guard.as_ref().map(|atlas| atlas.texture.clone())
                            };
                            let blurred_atlas_texture = {
                                let guard = self.blur_atlas.lock().unwrap();
                                guard.as_ref().map(|atlas| atlas.texture.clone())
                            };

                            if let (Some(source_atlas_texture), Some(blurred_atlas_texture)) =
                                (source_atlas_texture, blurred_atlas_texture)
                            {
                                post_enc.clear_texture(
                                    &source_atlas_texture,
                                    &wgpu::ImageSubresourceRange {
                                        aspect: wgpu::TextureAspect::All,
                                        base_mip_level: 0,
                                        mip_level_count: None,
                                        base_array_layer: 0,
                                        array_layer_count: None,
                                    },
                                );

                                let mut copied_indices = Vec::with_capacity(layout.placements.len());
                                for &(idx, ax, ay) in &layout.placements {
                                    let entry = &mut blurred_textures[idx];
                                    let (src_x, src_y, src_w, src_h) = entry.source_rect;
                                    let padding =
                                        ((entry.width as f32 - src_w) * 0.5).max(0.0) as u32;

                                    #[cfg(target_os = "android")]
                                    let src_origin_y = (h as f32 - src_y - src_h).max(0.0) as u32;
                                    #[cfg(not(target_os = "android"))]
                                    let src_origin_y = src_y.max(0.0) as u32;

                                    let src_origin_x = src_x.max(0.0) as u32;
                                    let copy_width = (src_w as u32)
                                        .min(w.saturating_sub(src_origin_x))
                                        .min(entry.width.saturating_sub(padding));
                                    let copy_height = (src_h as u32)
                                        .min(h.saturating_sub(src_origin_y))
                                        .min(entry.height.saturating_sub(padding));
                                    if copy_width == 0 || copy_height == 0 {
                                        entry.blur_valid = false;
                                        entry.blur_rebuild_pending = true;
                                        entry.atlas_valid = false;
                                        entry.atlas_dirty = true;
                                        continue;
                                    }

                                    post_enc.copy_texture_to_texture(
                                        wgpu::TexelCopyTextureInfo {
                                            texture: scene_texture,
                                            mip_level: 0,
                                            origin: wgpu::Origin3d {
                                                x: src_origin_x,
                                                y: src_origin_y,
                                                z: 0,
                                            },
                                            aspect: wgpu::TextureAspect::All,
                                        },
                                        wgpu::TexelCopyTextureInfo {
                                            texture: &source_atlas_texture,
                                            mip_level: 0,
                                            origin: wgpu::Origin3d {
                                                x: ax + padding,
                                                y: ay + padding,
                                                z: 0,
                                            },
                                            aspect: wgpu::TextureAspect::All,
                                        },
                                        wgpu::Extent3d {
                                            width: copy_width,
                                            height: copy_height,
                                            depth_or_array_layers: 1,
                                        },
                                    );
                                    copied_indices.push((idx, ax, ay));
                                }
                                atlas_wide_source_copies_this_frame = copied_indices.len();
                                stage_timer.mark("blur_copy_submit");

                                if !copied_indices.is_empty() {
                                    let result = pipeline.apply_frosted_glass_kawase(
                                        &mut post_enc,
                                        &source_atlas_texture,
                                        &blurred_atlas_texture,
                                        max_radius,
                                        None,
                                    );
                                    if let Err(e) = result {
                                        log::warn!(
                                            "[BlurAtlasWide] atlas-wide Kawase failed: {:?}",
                                            e
                                        );
                                    } else {
                                        for (idx, ax, ay) in copied_indices {
                                            if let Some(entry) = blurred_textures.get_mut(idx) {
                                                entry.blur_valid = true;
                                                entry.blur_rebuild_pending = false;
                                                entry.atlas_valid = true;
                                                entry.atlas_dirty = false;
                                                entry.atlas_x = ax;
                                                entry.atlas_y = ay;
                                                entry.last_blur_rebuild_frame = current_frame;
                                                if entry.dirty_kind != BlurDirtyKind::ChildrenChanged {
                                                    entry.dirty_kind = BlurDirtyKind::Clean;
                                                }
                                            }
                                        }
                                        for entry in blurred_textures.iter_mut() {
                                            if entry.blur_rebuild_pending {
                                                continue;
                                            }
                                            if matches!(
                                                entry.dirty_kind,
                                                BlurDirtyKind::OverlayOnlyChanged | BlurDirtyKind::Clean
                                            ) {
                                                entry.dirty_kind = BlurDirtyKind::Clean;
                                            }
                                        }
                                        atlas_wide_blur_valid_this_frame = true;
                                    }
                                }
                                stage_timer.mark("blur_render_submit");

                                if atlas_wide_blur_valid_this_frame
                                    && current_frame % DIAG_LOG_EVERY_N_FRAMES == 0
                                {
                                    log::info!(
                                        "[BlurAtlasWide] Frame {} — copied {} slots, atlas={}x{} slot={} gap={} radius={:.1}",
                                        current_frame,
                                        atlas_wide_source_copies_this_frame,
                                        layout.width,
                                        layout.height,
                                        layout.slot,
                                        layout.gap,
                                        max_radius,
                                    );
                                }
                            }
                        } else if current_frame % DIAG_LOG_EVERY_N_FRAMES == 0 {
                            log::info!(
                                "[BlurAtlasWide] fallback: placements={} radius_class_uniform={} budget_ok={} atlas={}x{}",
                                layout.placements.len(),
                                radius_class_uniform,
                                atlas_wide_within_budget,
                                layout.width,
                                layout.height
                            );
                        }
                    }
                }

                if !atlas_wide_blur_valid_this_frame {
                if self
                    .blur_atlas_wide_active_last_frame
                    .swap(false, std::sync::atomic::Ordering::Relaxed)
                {
                    for entry in blurred_textures.iter_mut() {
                        entry.blur_valid = false;
                        entry.blur_rebuild_pending = true;
                        entry.atlas_valid = false;
                        entry.atlas_dirty = true;
                    }
                }
                let mut rebuild_indices: Vec<usize> = blurred_textures
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| {
                        !entry.skipped_due_to_size
                            && blur_entry_visible(entry, w, h)
                            && (!entry.blur_valid
                                || entry.blur_rebuild_pending
                                || matches!(
                                    entry.dirty_kind,
                                    BlurDirtyKind::BackgroundChanged | BlurDirtyKind::BlurParamsChanged
                                ))
                    })
                    .map(|(idx, _)| idx)
                    .collect();
                rebuild_indices.sort_by_key(|&idx| {
                    let entry = &blurred_textures[idx];
                    (
                        entry.blur_valid,
                        entry.last_blur_rebuild_frame,
                        std::cmp::Reverse((entry.width as u64) * (entry.height as u64)),
                    )
                });
                let rebuild_pending = rebuild_indices.len();
                // Keep rebuild pressure bounded. When the cadence governor is
                // targeting 60Hz, prioritize frame stability over catching up
                // stale blur entries quickly; cached blur remains visually
                // acceptable and invalid entries are filled gradually.
                #[cfg(target_os = "android")]
                let rebuild_budget = MAX_BLUR_REBUILDS_PER_FRAME_AT_60HZ;
                #[cfg(not(target_os = "android"))]
                let rebuild_budget = MAX_BLUR_REBUILDS_PER_FRAME;
                if rebuild_indices.len() > rebuild_budget {
                    rebuild_indices.truncate(rebuild_budget);
                }
                if rebuild_pending > 0 && current_frame % DIAG_LOG_EVERY_N_FRAMES == 0 {
                    log::info!(
                        "[BlurLegacy] Budget: rebuilding {}/{} pending entries",
                        rebuild_indices.len(),
                        rebuild_pending
                    );
                }

                let mut blur_entries: Vec<(usize, u32, wgpu::Texture, f32)> = Vec::new();
                for idx in rebuild_indices {
                    let entry = &mut blurred_textures[idx];
                    if entry.skipped_due_to_size {
                        continue;
                    }

                    let (src_x, src_y, src_w, src_h) = entry.source_rect;
                    let padding = ((entry.width as f32 - src_w) * 0.5).max(0.0) as u32;

                    #[cfg(target_os = "android")]
                    let src_origin_y = (h as f32 - src_y - src_h).max(0.0) as u32;
                    #[cfg(not(target_os = "android"))]
                    let src_origin_y = src_y.max(0.0) as u32;

                    let src_origin_x = src_x.max(0.0) as u32;
                    let copy_width = (src_w as u32)
                        .min(w.saturating_sub(src_origin_x))
                        .min(entry.width.saturating_sub(padding));
                    let copy_height = (src_h as u32)
                        .min(h.saturating_sub(src_origin_y))
                        .min(entry.height.saturating_sub(padding));
                    if copy_width == 0 || copy_height == 0 {
                        continue;
                    }

                    post_enc.clear_texture(
                        &entry.texture,
                        &wgpu::ImageSubresourceRange {
                            aspect: wgpu::TextureAspect::All,
                            base_mip_level: 0,
                            mip_level_count: None,
                            base_array_layer: 0,
                            array_layer_count: None,
                        },
                    );
                    post_enc.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: scene_texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: src_origin_x,
                                y: src_origin_y,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &entry.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: padding,
                                y: padding,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width: copy_width,
                            height: copy_height,
                            depth_or_array_layers: 1,
                        },
                    );

                    if entry.blur_radius > 0.0 {
                        blur_entries.push((idx, entry.view_id, entry.texture.clone(), entry.blur_radius));
                    }
                }
                stage_timer.mark("blur_copy_submit");

                let mut rebuilt_indices = Vec::new();
                for (idx, view_id, texture, blur_radius) in blur_entries {
                    let pool_guard = self.texture_pool.lock().unwrap();
                    let result = pipeline.apply_frosted_glass_kawase(
                        &mut post_enc,
                        &texture,
                        &texture,
                        blur_radius,
                        pool_guard.as_ref(),
                    );
                    if let Err(e) = result {
                        log::warn!(
                            "[BlurLegacy] Kawase failed for view_id={}: {:?}",
                            view_id,
                            e
                        );
                    } else {
                        rebuilt_indices.push(idx);
                    }
                }

                for idx in rebuilt_indices {
                    if let Some(entry) = blurred_textures.get_mut(idx) {
                        entry.blur_valid = true;
                        entry.blur_rebuild_pending = false;
                        entry.atlas_dirty = true;
                        entry.last_blur_rebuild_frame = current_frame;
                        if entry.dirty_kind != BlurDirtyKind::ChildrenChanged {
                            entry.dirty_kind = BlurDirtyKind::Clean;
                        }
                    }
                }
                for entry in blurred_textures.iter_mut() {
                    if entry.blur_rebuild_pending {
                        continue;
                    }
                    if matches!(
                        entry.dirty_kind,
                        BlurDirtyKind::OverlayOnlyChanged | BlurDirtyKind::Clean
                    ) {
                        entry.dirty_kind = BlurDirtyKind::Clean;
                    }
                }
                stage_timer.mark("blur_render_submit");
                } else {
                    self.blur_atlas_wide_active_last_frame
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            } else {
                // No filter pipeline yet — record zero-time for consistent timing
                self.blur_atlas_wide_active_last_frame
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                stage_timer.mark("blur_copy_submit");
                stage_timer.mark("blur_render_submit");
            }
        } else {
            self.blur_atlas_wide_active_last_frame
                .store(false, std::sync::atomic::Ordering::Relaxed);
            stage_timer.mark("blur_copy_submit");
            stage_timer.mark("blur_render_submit");
        }

        stage_timer.mark("pass3_start");

        // === PASS 3: Render deferred children to per-entry local textures ===
        // Only run when there are actual blur entries with deferred children.
        if has_blur {
            let mut blurred_textures = self.blurred_textures.lock().unwrap();
            for entry in blurred_textures.iter_mut() {
                if !blur_entry_visible(entry, w, h) {
                    continue;
                }
                if entry.deferred_children.is_empty() {
                    continue;
                }
                if entry.children_bounds.2 <= 0.0 || entry.children_bounds.3 <= 0.0 {
                    continue;
                }
                // Skip Pass 3 when children haven't changed and we already have a cached texture.
                if entry.dirty_kind != BlurDirtyKind::ChildrenChanged
                    && entry.children_texture.is_some()
                {
                    continue;
                }

                let mut children_scene = Scene::new();

                let global_x = entry.source_rect.0 as f64;
                let global_y = entry.source_rect.1 as f64;
                let origin_offset = Vec2::new(entry.children_bounds.0 as f64, entry.children_bounds.1 as f64);

                for &child_id in &entry.deferred_children {
                    render_deferred_child(
                        child_id,
                        &node_map,
                        &mut children_scene,
                        Vec2::new(global_x, global_y),
                        origin_offset,
                        &self.glyph_run_cache,
                        &self.glyph_run_cache_stats,
                    );
                }

                let cw = entry.children_bounds.2.ceil() as u32;
                let ch = entry.children_bounds.3.ceil() as u32;

                let needs_new_children_texture = entry.children_texture.as_ref().map_or(true, |t| {
                    t.width() != cw || t.height() != ch
                });

                if needs_new_children_texture {
                    log::debug!(
                        "[Blur] Pass 3: Creating local children texture {}x{} for view_id={}",
                        cw, ch, entry.view_id
                    );
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Children Local Texture"),
                        size: wgpu::Extent3d {
                            width: cw,
                            height: ch,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::STORAGE_BINDING
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    });
                    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                    entry.children_texture = Some(texture);
                    entry.children_texture_view = Some(view);
                    entry.children_bind_group = None;
                    entry.last_children_uniform_data = None;
                    entry.last_children_overlay_data = None;
                }

                if let Some(ref view) = entry.children_texture_view {
                    if let Err(e) = renderer.render_to_texture(
                        device,
                        queue,
                        &children_scene,
                        view,
                        &vello::RenderParams {
                            base_color: Color::TRANSPARENT,
                            width: cw,
                            height: ch,
                            antialiasing_method: aa_config,
                        },
                    ) {
                        log::warn!(
                            "[Blur] Failed to render children texture for view_id={}: {:?}",
                            entry.view_id, e
                        );
                    } else {
                        #[cfg(not(target_arch = "wasm32"))]
                        if self.debug_frames_enabled() {
                            let frame_num = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            let debug_dir = self.debug_output_dir();
                            let path = debug_dir.join(format!(
                                "frame_{:06}_pass3_children_view_{}.png",
                                frame_num % 1000,
                                entry.view_id
                            ));
                            if let Some(ref tex) = entry.children_texture {
                                self.save_texture_to_png(device, queue, tex, path.to_str().unwrap());
                            }
                        }
                    }
                }
            }
        }
        stage_timer.mark("pass3_done");

        // === PASS 3.5: Pack valid legacy blur textures into an atlas for
        // instanced composite. This preserves the correct per-entry blurred
        // texture content while reducing Pass 4 from ~90 bind/draw calls to
        // one bind group + one instanced draw. If packing overflows, we fall
        // back to the legacy per-entry compositor below.
        let mut atlas_bind_group: Option<wgpu::BindGroup> = None;
        let mut atlas_instance_count: u32 = 0;
        let mut atlas_enabled_this_frame = false;
        if has_blur {
            let mut blurred_textures = self.blurred_textures.lock().unwrap();
            let gap = if atlas_wide_blur_valid_this_frame {
                BLUR_ATLAS_WIDE_GAP_PX
            } else {
                BLUR_ATLAS_LEGACY_GAP_PX
            };
            if let Some(layout) = compute_blur_atlas_layout(&blurred_textures, w, h, gap) {
                if layout.placements.len() >= 8 {
                    self.ensure_blur_instanced_resources(device, surface_format, layout.placements.len());
                    let atlas_recreated = self.ensure_blur_atlas_texture(device, layout.width, layout.height);
                    if atlas_recreated && !atlas_wide_blur_valid_this_frame {
                        for entry in blurred_textures.iter_mut() {
                            entry.atlas_valid = false;
                            entry.atlas_dirty = true;
                        }
                    }

                    let mut instances: Vec<BlurInstance> = Vec::with_capacity(layout.placements.len());
                    let mut atlas_copies = 0usize;
                    {
                        let atlas_guard = self.blur_atlas.lock().unwrap();
                        if let Some(atlas) = atlas_guard.as_ref() {
                            for &(idx, ax, ay) in &layout.placements {
                                let entry = &mut blurred_textures[idx];
                                let placement_changed =
                                    !entry.atlas_valid || entry.atlas_x != ax || entry.atlas_y != ay;
                                if placement_changed {
                                    entry.atlas_x = ax;
                                    entry.atlas_y = ay;
                                    entry.atlas_valid = atlas_wide_blur_valid_this_frame;
                                    entry.atlas_dirty = !atlas_wide_blur_valid_this_frame;
                                }
                                if !entry.blur_valid {
                                    continue;
                                }

                                if atlas_wide_blur_valid_this_frame {
                                    entry.atlas_x = ax;
                                    entry.atlas_y = ay;
                                    entry.atlas_valid = true;
                                    entry.atlas_dirty = false;
                                } else if entry.atlas_dirty || !entry.atlas_valid {
                                    post_enc.copy_texture_to_texture(
                                        wgpu::TexelCopyTextureInfo {
                                            texture: &entry.texture,
                                            mip_level: 0,
                                            origin: wgpu::Origin3d::ZERO,
                                            aspect: wgpu::TextureAspect::All,
                                        },
                                        wgpu::TexelCopyTextureInfo {
                                            texture: &atlas.texture,
                                            mip_level: 0,
                                            origin: wgpu::Origin3d { x: ax, y: ay, z: 0 },
                                            aspect: wgpu::TextureAspect::All,
                                        },
                                        wgpu::Extent3d {
                                            width: entry.width,
                                            height: entry.height,
                                            depth_or_array_layers: 1,
                                        },
                                    );
                                    entry.atlas_dirty = false;
                                    entry.atlas_valid = true;
                                    atlas_copies += 1;
                                }

                                let mat = entry.transform.as_coeffs();
                                let overlay_color = entry.overlay_color;
                                instances.push(BlurInstance {
                                    rect: [
                                        mat[4] as f32,
                                        mat[5] as f32,
                                        entry.width as f32,
                                        entry.height as f32,
                                    ],
                                    source_rect: [
                                        entry.atlas_x as f32,
                                        entry.atlas_y as f32,
                                        entry.width as f32,
                                        entry.height as f32,
                                    ],
                                    color: [
                                        overlay_color.components[0],
                                        overlay_color.components[1],
                                        overlay_color.components[2],
                                        overlay_color.components[3],
                                    ],
                                    params: [
                                        entry.border_radius as f32,
                                        entry.opacity,
                                        if entry.blur_style == 1 || entry.blur_style == 3 { 1.0 } else { 0.0 },
                                        0.0,
                                    ],
                                });
                            }

                            let frame = BlurFrameUniform {
                                viewport_size: [w as f32, h as f32],
                                _pad: [0.0, 0.0],
                            };
                            if let Some(frame_buffer) = self.blur_frame_uniform.lock().unwrap().as_ref() {
                                queue.write_buffer(frame_buffer, 0, bytemuck::bytes_of(&frame));
                            }
                            if let Some(instance_buffer) = self.blur_instance_buffer.lock().unwrap().as_ref() {
                                queue.write_buffer(instance_buffer, 0, bytemuck::cast_slice(&instances));
                            }

                            let layout_guard = self.blur_instanced_bind_group_layout.lock().unwrap();
                            if let (Some(layout), Some(frame_buffer), Some(instance_buffer)) = (
                                layout_guard.as_ref(),
                                self.blur_frame_uniform.lock().unwrap().as_ref(),
                                self.blur_instance_buffer.lock().unwrap().as_ref(),
                            ) {
                                let mut cached_bind_group =
                                    self.blur_instanced_bind_group.lock().unwrap();
                                if cached_bind_group.is_none() {
                                    let sampler = self.sampler.lock().unwrap();
                                    let sampler =
                                        sampler.as_ref().expect("Sampler should be initialized");
                                    *cached_bind_group = Some(device.create_bind_group(
                                        &wgpu::BindGroupDescriptor {
                                            label: Some("Blur Atlas Instanced Bind Group"),
                                            layout,
                                            entries: &[
                                                wgpu::BindGroupEntry {
                                                    binding: 0,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        &atlas.view,
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 1,
                                                    resource: wgpu::BindingResource::Sampler(sampler),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 2,
                                                    resource: frame_buffer.as_entire_binding(),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 3,
                                                    resource: instance_buffer.as_entire_binding(),
                                                },
                                            ],
                                        },
                                    ));
                                }
                                atlas_bind_group = cached_bind_group.clone();
                                atlas_instance_count = instances.len() as u32;
                                atlas_enabled_this_frame = atlas_instance_count > 0;
                            }
                        }
                    }
                    if atlas_wide_blur_valid_this_frame && !atlas_enabled_this_frame {
                        for entry in blurred_textures.iter_mut() {
                            entry.blur_valid = false;
                            entry.blur_rebuild_pending = true;
                            entry.atlas_valid = false;
                            entry.atlas_dirty = true;
                        }
                    }
                    if atlas_enabled_this_frame && FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed) % DIAG_LOG_EVERY_N_FRAMES == 0 {
                        log::info!(
                            "[BlurAtlas] compositing {} {} blur entries via atlas {}x{} slot={} gap={}, copies={}",
                            atlas_instance_count,
                            if atlas_wide_blur_valid_this_frame { "atlas-wide" } else { "legacy" },
                            layout.width,
                            layout.height,
                            layout.slot,
                            layout.gap,
                            if atlas_wide_blur_valid_this_frame {
                                atlas_wide_source_copies_this_frame
                            } else {
                                atlas_copies
                            }
                        );
                    }
                }
            }
        }

        // Surface texture was already acquired by GraphicsRuntime::begin_frame().
        // No texture wait happens inside the backend render path.
        stage_timer.mark("surface_ready");

        // === PASS 4: Final Blit ===
        // Determine render target (capture texture for debug, else surface directly)
        #[cfg(not(target_arch = "wasm32"))]
        let debug_frame_num = if self.debug_frames_enabled() {
            let frame_num = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Some(frame_num % 1000)
        } else {
            None
        };

        #[cfg(not(target_arch = "wasm32"))]
        let capture_texture = if self.debug_frames_enabled() && debug_frame_num.is_some() {
            let capture_tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Capture Texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: surface_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            Some(capture_tex)
        } else {
            None
        };

        #[cfg(not(target_arch = "wasm32"))]
        let render_target_view = if let Some(ref capture_tex) = capture_texture {
            capture_tex.create_view(&Default::default())
        } else {
            target_view.clone()
        };
        #[cfg(target_arch = "wasm32")]
        let render_target_view = target_view.clone();

        #[allow(unused_assignments)]
        let mut had_blur_textures = false;
        {
            // Ensure blit pipeline matches the surface format (e.g. Bgra8Unorm on macOS)
            self.ensure_blit_pipeline(device, surface_format);

            let mut rp = post_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Vello Blit Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &render_target_view,
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
            let blit_pipeline_guard = self.blit_pipeline.lock().unwrap();
            let blit_pipeline = blit_pipeline_guard.as_ref().unwrap();
            rp.set_pipeline(blit_pipeline);
            rp.set_bind_group(0, &tb.current().bind_group, &[]);
            rp.draw(0..3, 0..1);
            drop(blit_pipeline_guard);

            // Draw blurred textures using composite pipeline (skip when neither blur nor cache draws exist)
            if has_blur || !cached_draws.is_empty() {
                log::debug!("[Blur Pass 4] About to lock blurred_textures for compositing");
                let mut blurred_textures = self.blurred_textures.lock().unwrap();
                log::debug!(
                    "[Blur Pass 4] Locked blurred_textures, count = {}",
                    blurred_textures.len()
                );
                log::debug!(
                    "[Blur] COMPOSITING {} blurred textures, {} cached draws",
                    blurred_textures.len(),
                    cached_draws.len()
                );

                log::debug!("[Blur] Surface config format: {:?}", surface_format);

                let needs_pipeline = self.blur_composite_pipeline.lock().unwrap().is_none();
                log::debug!("[Blur] needs_pipeline = {}", needs_pipeline);
                if needs_pipeline {
                    log::debug!(
                        "[Blur] Creating composite pipeline with surface format {:?}",
                        surface_format
                    );
                    self.create_blur_composite_pipeline(device, surface_format);
                    log::debug!("[Blur] Pipeline creation complete");
                }

                let blur_pipeline = self.blur_composite_pipeline.lock().unwrap();
                let blur_bg_layout = self.blur_composite_bind_group_layout.lock().unwrap();
                let uniform_buffer = self.blur_composite_uniforms.lock().unwrap();
                let overlay_uniform_buffer = self.blur_composite_overlay_uniforms.lock().unwrap();
                log::debug!("[Blur] Got all locks");

                let pipeline_ready = blur_pipeline.is_some();
                let layout_ready = blur_bg_layout.is_some();
                let uniforms_ready = uniform_buffer.is_some();
                let overlay_ready = overlay_uniform_buffer.is_some();

                if !(pipeline_ready && layout_ready && uniforms_ready && overlay_ready) {
                    log::warn!(
                        "[Blur] Resources not ready: pipeline={}, layout={}, uniforms={}, overlay={}",
                        pipeline_ready,
                        layout_ready,
                        uniforms_ready,
                        overlay_ready
                    );
                }

                if let (Some(pipeline), Some(layout), _, _) = (
                    blur_pipeline.as_ref(),
                    blur_bg_layout.as_ref(),
                    uniform_buffer.as_ref(),
                    overlay_uniform_buffer.as_ref(),
                ) {
                    let sampler = self.sampler.lock().unwrap();
                    let sampler = sampler.as_ref().expect("Sampler should be initialized");

                    // === Draw cached subtrees first (before blur composite) ===
                    if !cached_draws.is_empty() {
                        log::debug!("[RasterCache] Drawing {} cached subtrees", cached_draws.len());
                        let staging_buffer = self.blur_staging_buffer.lock().unwrap();
                        let staging = staging_buffer
                            .as_ref()
                            .expect("blur staging buffer not initialized");
                        let alignment = *self.blur_staging_alignment.lock().unwrap();
                        let stride = alignment * 2;

                        let scale_x = 2.0 / w as f32;
                        let scale_y = -2.0 / h as f32;
                        let offset_x = -1.0;
                        let offset_y = 1.0;

                        for draw in &cached_draws {
                            let affine = draw.transform;
                            let mat = affine.as_coeffs();
                            let tex_width = draw.width;
                            let tex_height = draw.height;

                            let uniform_data: [f32; 12] = [
                                mat[0] as f32 * tex_width * scale_x,
                                mat[2] as f32 * tex_width * scale_x,
                                0.0,
                                0.0,
                                mat[1] as f32 * tex_height * scale_y,
                                mat[3] as f32 * tex_height * scale_y,
                                0.0,
                                0.0,
                                mat[4] as f32 * scale_x + offset_x,
                                mat[5] as f32 * scale_y + offset_y,
                                1.0,
                                0.0,
                            ];

                            let base_offset = self
                                .blur_staging_offset
                                .fetch_add(stride, std::sync::atomic::Ordering::Relaxed);
                            if base_offset + stride > 1024 * 1024 {
                                log::warn!(
                                    "[RasterCache] Staging buffer overflow, skipping remaining draws"
                                );
                                break;
                            }

                            let overlay_data: [f32; 12] = [
                                0.0, 0.0, 0.0, 0.0,
                                0.0, tex_width, tex_height, 0.0,
                                0.0, 0.0, 0.0, 0.0,
                            ];

                            queue.write_buffer(
                                staging,
                                base_offset as u64,
                                bytemuck::cast_slice(&uniform_data),
                            );
                            queue.write_buffer(
                                staging,
                                (base_offset + alignment) as u64,
                                bytemuck::cast_slice(&overlay_data),
                            );

                            let gpu_pool = self.gpu_texture_pool.lock().unwrap();
                            if let Some(pool) = gpu_pool.as_ref() {
                                if let Some(ptex) = pool.get_texture(draw.texture_id) {
                                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                                        label: Some("RasterCache Composite Bind Group"),
                                        layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: wgpu::BindingResource::TextureView(ptex.view()),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::Sampler(&sampler),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                                    buffer: staging,
                                                    offset: base_offset as u64,
                                                    size: Some(std::num::NonZeroU64::new(48).unwrap()),
                                                }),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                                    buffer: staging,
                                                    offset: (base_offset + alignment) as u64,
                                                    size: Some(std::num::NonZeroU64::new(48).unwrap()),
                                                }),
                                            },
                                        ],
                                    });
                                    rp.set_pipeline(pipeline);
                                    rp.set_bind_group(0, &bind_group, &[]);
                                    rp.draw(0..6, 0..1);
                                }
                            }
                        }
                    }

                    // Full downgrade: no instanced/global backdrop composite.
                    // Draw each per-entry blurred texture with the legacy
                    // composite shader so actual blurred pixels are visible.

                    let scale_x = 2.0 / w as f32;
                    let scale_y = -2.0 / h as f32;
                    let offset_x = -1.0;
                    let offset_y = 1.0;

                    // Set pipeline once before the per-entry children loop
                    rp.set_pipeline(pipeline);

                    if atlas_enabled_this_frame {
                        if let Some(ref bind_group) = atlas_bind_group {
                            let atlas_pipeline_guard = self.blur_instanced_pipeline.lock().unwrap();
                            if let Some(atlas_pipeline) = atlas_pipeline_guard.as_ref() {
                                rp.set_pipeline(atlas_pipeline);
                                rp.set_bind_group(0, bind_group, &[]);
                                rp.draw(0..6, 0..atlas_instance_count);
                                rp.set_pipeline(pipeline);
                            }
                        }
                    }

                    let backdrop_guard = self.backdrop_blur.lock().unwrap();
                    let backdrop_view = if USE_FULL_FRAME_BACKDROP_BLUR {
                        backdrop_guard.as_ref().map(|b| &b.view)
                    } else {
                        None
                    };

                    for entry in blurred_textures.iter_mut() {
                        let is_visible = blur_entry_visible(entry, w, h);
                        let blur_drawn_by_atlas =
                            atlas_enabled_this_frame && entry.blur_valid && entry.atlas_valid;
                        if !blur_drawn_by_atlas
                            && !entry.skipped_due_to_size
                            && entry.blur_valid
                            && is_visible
                        {
                            let affine = entry.transform;
                            let mat = affine.as_coeffs();
                            let tex_width = entry.width as f32;
                            let tex_height = entry.height as f32;

                            let uniform_data: [f32; 12] = [
                                mat[0] as f32 * tex_width * scale_x,
                                mat[2] as f32 * tex_width * scale_x,
                                0.0,
                                0.0,
                                mat[1] as f32 * tex_height * scale_y,
                                mat[3] as f32 * tex_height * scale_y,
                                0.0,
                                0.0,
                                mat[4] as f32 * scale_x + offset_x,
                                mat[5] as f32 * scale_y + offset_y,
                                entry.opacity,
                                0.0,
                            ];
                            let overlay_color = entry.overlay_color;
                            // source_width/source_height are zero here, so
                            // blur_composite.wgsl samples local texture UVs
                            // instead of the now-disabled backdrop path.
                            #[cfg(target_os = "android")]
                            let backdrop_source_y = (h as f32
                                - entry.source_rect.1
                                - entry.source_rect.3)
                                .max(0.0);
                            #[cfg(not(target_os = "android"))]
                            let backdrop_source_y = entry.source_rect.1;
                            let (source_x, source_y, source_w, source_h) =
                                if USE_FULL_FRAME_BACKDROP_BLUR && backdrop_view.is_some() {
                                    (
                                        entry.source_rect.0,
                                        backdrop_source_y,
                                        entry.source_rect.2,
                                        entry.source_rect.3,
                                    )
                                } else {
                                    (0.0, 0.0, 0.0, 0.0)
                                };
                            let overlay_data: [f32; 12] = [
                                overlay_color.components[0],
                                overlay_color.components[1],
                                overlay_color.components[2],
                                overlay_color.components[3],
                                entry.border_radius as f32,
                                entry.width as f32,
                                entry.height as f32,
                                if entry.blur_style == 1 || entry.blur_style == 3 {
                                    1.0
                                } else {
                                    0.0
                                },
                                source_x,
                                source_y,
                                source_w,
                                source_h,
                            ];

                            if entry.composite_uniform_buffer.is_none() {
                                entry.composite_uniform_buffer = Some(device.create_buffer(
                                    &wgpu::BufferDescriptor {
                                        label: Some("Blur Legacy Composite Uniform Buffer"),
                                        size: 48,
                                        usage: wgpu::BufferUsages::UNIFORM
                                            | wgpu::BufferUsages::COPY_DST,
                                        mapped_at_creation: false,
                                    },
                                ));
                            }
                            if entry.composite_overlay_buffer.is_none() {
                                entry.composite_overlay_buffer = Some(device.create_buffer(
                                    &wgpu::BufferDescriptor {
                                        label: Some("Blur Legacy Composite Overlay Buffer"),
                                        size: 48,
                                        usage: wgpu::BufferUsages::UNIFORM
                                            | wgpu::BufferUsages::COPY_DST,
                                        mapped_at_creation: false,
                                    },
                                ));
                            }
                            let uniform_buffer =
                                entry.composite_uniform_buffer.as_ref().unwrap();
                            let overlay_buffer =
                                entry.composite_overlay_buffer.as_ref().unwrap();
                            if entry.last_composite_uniform_data != Some(uniform_data) {
                                queue.write_buffer(
                                    uniform_buffer,
                                    0,
                                    bytemuck::cast_slice(&uniform_data),
                                );
                                entry.last_composite_uniform_data = Some(uniform_data);
                            }
                            if entry.last_composite_overlay_data != Some(overlay_data) {
                                queue.write_buffer(
                                    overlay_buffer,
                                    0,
                                    bytemuck::cast_slice(&overlay_data),
                                );
                                entry.last_composite_overlay_data = Some(overlay_data);
                            }

                            let uses_backdrop =
                                USE_FULL_FRAME_BACKDROP_BLUR && backdrop_view.is_some();
                            if entry.composite_bind_group.is_none()
                                || entry.composite_uses_backdrop != uses_backdrop
                            {
                                let source_view = backdrop_view.unwrap_or(&entry.texture_view);
                                entry.composite_bind_group = Some(device.create_bind_group(
                                    &wgpu::BindGroupDescriptor {
                                        label: Some(&format!(
                                            "Blur Legacy Composite Bind Group {}",
                                            entry.view_id
                                        )),
                                        layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: wgpu::BindingResource::TextureView(
                                                    source_view,
                                                ),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::Sampler(&sampler),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: uniform_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: overlay_buffer.as_entire_binding(),
                                            },
                                        ],
                                    },
                                ));
                                entry.composite_uses_backdrop = uses_backdrop;
                            }
                            rp.set_bind_group(0, entry.composite_bind_group.as_ref().unwrap(), &[]);
                            rp.draw(0..6, 0..1);
                        }

                        if !is_visible || entry.children_texture_view.is_none() {
                            continue;
                        }
                        // === Draw per-entry children overlay ===
                        if let Some(ref children_view) = entry.children_texture_view {
                            let bx = entry.children_bounds.0 as f64;
                            let by = entry.children_bounds.1 as f64;
                            let bw = entry.children_bounds.2 as f64;
                            let bh = entry.children_bounds.3 as f64;
                            let children_transform = Affine::translate((bx, by));
                            let cmat = children_transform.as_coeffs();
                            let ctex_width = bw as f32;
                            let ctex_height = bh as f32;
                            let children_uniform_data: [f32; 12] = [
                                cmat[0] as f32 * ctex_width * scale_x,
                                cmat[2] as f32 * ctex_width * scale_x,
                                0.0,
                                0.0,
                                cmat[1] as f32 * ctex_height * scale_y,
                                cmat[3] as f32 * ctex_height * scale_y,
                                0.0,
                                0.0,
                                cmat[4] as f32 * scale_x + offset_x,
                                cmat[5] as f32 * scale_y + offset_y,
                                1.0,
                                0.0,
                            ];
                            let children_overlay_data: [f32; 12] = [
                                0.0, 0.0, 0.0, 0.0,
                                0.0,
                                ctex_width,
                                ctex_height,
                                0.0,
                                0.0, 0.0, 0.0, 0.0,
                            ];
                            if entry.children_uniform_buffer.is_none() {
                                entry.children_uniform_buffer = Some(device.create_buffer(
                                    &wgpu::BufferDescriptor {
                                        label: Some("Blur Children Composite Uniform Buffer"),
                                        size: 48,
                                        usage: wgpu::BufferUsages::UNIFORM
                                            | wgpu::BufferUsages::COPY_DST,
                                        mapped_at_creation: false,
                                    },
                                ));
                            }
                            if entry.children_overlay_buffer.is_none() {
                                entry.children_overlay_buffer = Some(device.create_buffer(
                                    &wgpu::BufferDescriptor {
                                        label: Some("Blur Children Composite Overlay Buffer"),
                                        size: 48,
                                        usage: wgpu::BufferUsages::UNIFORM
                                            | wgpu::BufferUsages::COPY_DST,
                                        mapped_at_creation: false,
                                    },
                                ));
                            }
                            let children_uniform_buffer =
                                entry.children_uniform_buffer.as_ref().unwrap();
                            let children_overlay_buffer =
                                entry.children_overlay_buffer.as_ref().unwrap();
                            if entry.last_children_uniform_data != Some(children_uniform_data) {
                                queue.write_buffer(
                                    children_uniform_buffer,
                                    0,
                                    bytemuck::cast_slice(&children_uniform_data),
                                );
                                entry.last_children_uniform_data = Some(children_uniform_data);
                            }
                            if entry.last_children_overlay_data != Some(children_overlay_data) {
                                queue.write_buffer(
                                    children_overlay_buffer,
                                    0,
                                    bytemuck::cast_slice(&children_overlay_data),
                                );
                                entry.last_children_overlay_data = Some(children_overlay_data);
                            }
                            if entry.children_bind_group.is_none() {
                                entry.children_bind_group = Some(device.create_bind_group(
                                    &wgpu::BindGroupDescriptor {
                                        label: Some(&format!(
                                            "Children Composite Bind Group {}",
                                            entry.view_id
                                        )),
                                        layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: wgpu::BindingResource::TextureView(
                                                    children_view,
                                                ),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::Sampler(&sampler),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: children_uniform_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: children_overlay_buffer.as_entire_binding(),
                                            },
                                        ],
                                    },
                                ));
                            }
                            rp.set_bind_group(0, entry.children_bind_group.as_ref().unwrap(), &[]);
                            rp.draw(0..6, 0..1);
                            log::debug!(
                                "[Blur Pass 4] Drew children for view_id={}",
                                entry.view_id
                            );
                        }
                    }
                }
            had_blur_textures = !blurred_textures.is_empty();
            log::debug!(
                "[Blur Pass 4] Composited {} blur textures, had_blur_textures = {}",
                blurred_textures.len(),
                had_blur_textures
            );
            drop(blurred_textures);
        } else {
            log::debug!("[Blur Pass 4] Skipped compositing (no blur, no cached draws)");
        }
        }

        // If using capture texture, blit it to surface before present (same encoder)
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(ref capture_tex) = capture_texture {
            self.ensure_blit_pipeline(device, surface_format);
            let capture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Capture Blit Bind Group"),
                layout: self
                    .blit_bind_group_layout
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap(),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &capture_tex.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(
                            self.sampler.lock().unwrap().as_ref().unwrap(),
                        ),
                    },
                ],
            });

            {
                let mut rp = post_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Capture Blit Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                let blit_pipeline_guard = self.blit_pipeline.lock().unwrap();
                let blit_pipeline = blit_pipeline_guard.as_ref().unwrap();
                rp.set_pipeline(blit_pipeline);
                rp.set_bind_group(0, &capture_bind_group, &[]);
                rp.draw(0..3, 0..1);
            }
        }

        // Single submit for all post-Vello GPU work
        queue.submit(Some(post_enc.finish()));
        stage_timer.mark("blit_submit");

        // Debug: Save composite frame when we have blur textures
        #[cfg(not(target_arch = "wasm32"))]
        {
            log::debug!("[Debug] Checking had_blur_textures = {}", had_blur_textures);
            if had_blur_textures && self.debug_frames_enabled() {
                if let Some(capture_tex) = &capture_texture {
                    let debug_dir = self.debug_output_dir();
                    let frame_num = debug_frame_num.unwrap_or(0);
                    let capture_path =
                        debug_dir.join(format!("frame_{:06}_pass0_composite.png", frame_num));
                    log::debug!(
                        "[DebugSave disabled] Would save composite frame to {:?}",
                        capture_path
                    );
                    self.save_texture_to_png(
                        device,
                        queue,
                        capture_tex,
                        capture_path.to_str().unwrap(),
                    );
                }
            }
        }

        // Present is handled by GraphicsRuntime::end_frame (outside backend).
        stage_timer.mark("render_return");

        // After first successful render, save the pipeline cache
        static FIRST_RENDER_DONE: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if !FIRST_RENDER_DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
            log::info!("[ColdStart] First render completed, saving pipeline cache");
            self.save_cache();
        }

        // Log detailed frame timing and performance stats for diagnostics
        let _pacer_wait_ms = *self.pacer_wait_ms.lock().unwrap();
        let frame_interval_ms = *self.frame_interval_ms.lock().unwrap();
        let perf_stats = self.frame_perf_stats.lock().unwrap();
        let stats = self.perf_monitor.lock().unwrap().get_stats();
        {
            let report = stage_timer.report();

            let state_lock_time =
                report.get("init_check_to_perf_start") + report.get("perf_start_to_state_lock");
            let scene_build_time = report.get("state_lock_to_scene_build");
            let bake_time = report.get("scene_build_to_bake_done");
            let gpu_time = report.get("bake_done_to_gpu_render");
            let blur_copy_time = report.get("gpu_render_to_blur_copy_submit");
            let blur_render_time = report.get("blur_copy_submit_to_blur_render_submit");
            let pass3_time = report.get("blur_render_submit_to_pass3_done");
            // Surface texture is acquired in GraphicsRuntime::begin_frame, not inside backend.
            let get_texture_time = 0.0;
            let texture_wait_time = 0.0;
            let blit_time = report.get("surface_ready_to_blit_submit");
            let submit_return_time = report.get("blit_submit_to_render_return");
            let total = frame_start.elapsed().as_secs_f32() * 1000.0;

            if stats.total_frames % DIAG_LOG_EVERY_N_FRAMES == 0 || total > 18.0 {
                log::info!(
                    "[DIAG-BACKEND] Frame {}: Total={:.2}ms, UI={:.1}fps, Raster={:.1}fps, Target={:.1}fps, Jank={}({:.1}%), Drop={}({:.1}%) | State={:.2}ms, Scene={:.2}ms, Bake={:.2}ms, GPU={:.2}ms, BlurCopy={:.2}ms, BlurRender={:.2}ms, Pass3={:.2}ms, GetTex={:.2}ms, TexWait={:.2}ms, Blit={:.2}ms, SubmitReturn={:.2}ms, Interval={:.2}ms",
                    stats.total_frames,
                    total,
                    perf_stats.ui_fps,
                    perf_stats.raster_fps,
                    perf_stats.target_fps,
                    perf_stats.jank_count,
                    perf_stats.jank_rate * 100.0,
                    perf_stats.dropped_count,
                    perf_stats.drop_rate * 100.0,
                    state_lock_time,
                    scene_build_time,
                    bake_time,
                    gpu_time,
                    blur_copy_time,
                    blur_render_time,
                    pass3_time,
                    get_texture_time,
                    texture_wait_time,
                    blit_time,
                    submit_return_time,
                    frame_interval_ms,
                );
            }

            // Shadow cache DIAG logging
            if stats.total_frames % 60 == 0 {
                let cache_stats = self.shadow_cache_stats.lock().unwrap();
                let cache_size = self.shadow_cache.lock().unwrap().len();
                let total = cache_stats.hits + cache_stats.misses;
                if total > 0 {
                    log::info!(
                        "[DIAG] ShadowCache: size={} hits={} misses={} hit_rate={:.1}% evictions={}",
                        cache_size,
                        cache_stats.hits,
                        cache_stats.misses,
                        (cache_stats.hits as f64 / total as f64) * 100.0,
                        cache_stats.evictions
                    );
                }
            }

            // Glyph run cache DIAG logging
            if stats.total_frames % 60 == 0 {
                let cache_stats = self.glyph_run_cache_stats.lock().unwrap();
                let cache_size = self.glyph_run_cache.lock().unwrap().len();
                let total = cache_stats.hits + cache_stats.misses;
                if total > 0 {
                    log::info!(
                        "[DIAG] GlyphCache: size={} hits={} misses={} hit_rate={:.1}% evictions={}",
                        cache_size,
                        cache_stats.hits,
                        cache_stats.misses,
                        (cache_stats.hits as f64 / total as f64) * 100.0,
                        cache_stats.evictions
                    );
                }
            }

            if stats.total_frames % 300 == 0 && log::log_enabled!(log::Level::Debug) {
                report.print();
            }
        }

        Ok(())
    }

    /// Render a package using a pre-acquired surface texture (double-layer API entry point).
    ///
    /// The caller (e.g. `GraphicsRuntime::end_frame`) is responsible for presenting the surface texture.
    pub fn render_with_surface_texture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_texture: &wgpu::SurfaceTexture,
        surface_format: wgpu::TextureFormat,
        package: &dyxel_render_api::RenderPackage,
    ) -> RenderResult {
        let target_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_internal_impl(device, queue, &target_view, surface_format, package)
    }

    /// Render a package into a caller-owned texture view.
    ///
    /// This is used by the macOS offscreen-first architecture: the runtime
    /// renders the full Vello/composite result into an offscreen target and
    /// acquires the actual surface drawable only in `end_frame`.
    pub fn render_to_view(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_view: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        package: &dyxel_render_api::RenderPackage,
    ) -> RenderResult {
        self.render_internal_impl(device, queue, target_view, target_format, package)
    }

    /// Public entry point: acquires cached_textures lock once, then delegates to internal recursive renderer.
    fn render_node_recursive_with_transform(
        &self,
        id: u32,
        nodes: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
        scene: &mut Scene,
        parent_pos: Vec2,
        transform: Affine,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut vello::Renderer,
        filter_pipeline: Option<&crate::filter_pipeline::FilterPipeline>,
        blurred_textures: &mut Vec<BlurredTextureEntry>,
        cached_draws: &mut Vec<CachedDraw>,
        in_blur_subtree: bool,
        blur_scene_frame: u64,
    ) {
        let cache_guard = self.cached_textures.lock().unwrap();
        self.render_node_recursive_internal(
            id,
            nodes,
            scene,
            parent_pos,
            transform,
            device,
            queue,
            renderer,
            filter_pipeline,
            blurred_textures,
            &*cache_guard,
            cached_draws,
            in_blur_subtree,
            blur_scene_frame,
        );
    }
}

// =============================================================================
// Platform Coordinate System Correction
// =============================================================================

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

/// Returns the platform-specific coordinate correction transform.
#[inline]
pub fn platform_correction(viewport_height: f64) -> Affine {
    #[cfg(target_os = "android")]
    {
        // Android: Vello renders Y-up, need flip to match screen Y-down
        Affine::translate((0.0, viewport_height)) * Affine::scale_non_uniform(1.0, -1.0)
    }
    #[cfg(not(target_os = "android"))]
    {
        // macOS/iOS: Vello's render_to_texture already produces Y-down output
        let _ = viewport_height;
        Affine::IDENTITY
    }
}

/// Render node content with blur effect applied (Two-pass frosted glass)
///
/// In the two-pass approach:
/// 1. First pass: Render all content to scene texture (done by caller)
/// 2. Second pass: Sample from scene texture, apply blur, overlay color
///
/// This function prepares the blur entry for the second pass.
fn render_with_blur(
    blur: &dyxel_render_api::BlurEffect,
    id: u32,
    nodes: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
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
    blur_scene_frame: u64,
) -> bool {
    // Unused imports - kept for reference but not needed in two-pass approach
    // use vello::peniko::{Fill, Color};
    // use kurbo::{Rect as KRect, RoundedRect};

    // Calculate padded size for blur (need extra space for blur bleed)
    let blur_radius = blur.blur_radius as f64;
    let padding = (blur_radius * 2.5).ceil() as u32;

    // ── LOD selection: sample from a lower-res backdrop pyramid level ──
    let backdrop_lod: u8 = if blur_radius <= 4.0 {
        1 // half-res
    } else if blur_radius <= 12.0 {
        2 // quarter-res
    } else {
        3 // eighth-res
    };
    // Keep blur texture at full size — only the source backdrop is downsampled.
    // This avoids transform/padding mismatch between render_with_blur and Pass 2.
    let content_width_px = quantize_blur_size_px(node_width as f32).max(1.0) as u32;
    let content_height_px = quantize_blur_size_px(node_height as f32).max(1.0) as u32;
    let texture_width = (content_width_px + padding * 2).max(1);
    let texture_height = (content_height_px + padding * 2).max(1);

    // Check if we already have an entry for this view_id (caching)
    let existing_index = blurred_textures.iter().position(|e| e.view_id == id);
    let bucket_alloc_width = blur_texture_alloc_extent_px(texture_width);
    let bucket_alloc_height = blur_texture_alloc_extent_px(texture_height);
    let (allocated_texture_width, allocated_texture_height) =
        existing_index.map_or((bucket_alloc_width, bucket_alloc_height), |idx| {
            let entry = &blurred_textures[idx];
            (
                entry.allocated_width.max(bucket_alloc_width),
                entry.allocated_height.max(bucket_alloc_height),
            )
        });
    let needs_new_texture = existing_index.map_or(true, |idx| {
        let entry = &blurred_textures[idx];
        entry.allocated_width < texture_width || entry.allocated_height < texture_height
    });

    let offscreen_texture = if needs_new_texture {
        // Create offscreen texture for the blurred result. The physical texture
        // is bucketed; entry.width/height remain the exact active draw rect.
        let texture_desc = wgpu::TextureDescriptor {
            label: Some("Blur Offscreen Texture"),
            size: wgpu::Extent3d {
                width: allocated_texture_width,
                height: allocated_texture_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let tex = device.create_texture(&texture_desc);
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        Some((tex, view))
    } else {
        None
    };

    // NOTE: For true two-pass frosted glass, we don't render anything here.
    // The blur texture will be created AFTER the main scene is rendered,
    // by sampling from the scene texture and applying blur.
    //
    // This ensures we blur the actual background content, not a temp scene.
    //
    // Flow:
    // 1. Scene building: record blur view info (position, size, etc.)
    // 2. render_to_texture: render main scene
    // 3. Post-process: for each blur view, sample from scene texture, blur it
    // 4. Blit: draw scene, then blurred textures, then deferred children

    // Store the blurred texture for compositing in the final blit pass
    // Adjust transform to account for the padding offset
    let final_transform =
        local_transform * Affine::translate((-(padding as f64), -(padding as f64)));

    // Calculate source rectangle in scene coordinates for two-pass rendering
    // This will be used in the second pass to sample from the scene texture
    // Note: On macOS/iOS, Taffy Y-down needs to be converted to Vello Y-up for correct sampling
    let source_x = quantize_blur_pos_px(local_transform.as_coeffs()[4] as f32); // translation x
    let source_y_taffy = quantize_blur_pos_px(local_transform.as_coeffs()[5] as f32); // translation y (Taffy Y-down)

    // Get viewport height from scene transform (stored in _state)
    // For Y-down to Y-up conversion: vello_y = viewport_height - taffy_y - node_height
    // But we need viewport height which isn't directly available here
    // Instead, we'll store the Taffy Y value and let the copy code handle the conversion

    // Collect deferred children - they will be rendered after the blurred background
    let deferred_children: Vec<u32> = blur.deferred_children.clone();

    // Compute bounding box of deferred children for local rendering
    let mut children_bounds_rect: Option<kurbo::Rect> = None;
    for &child_id in &deferred_children {
        if let Some(bounds) = compute_subtree_bounds(
            child_id,
            nodes,
            Vec2::new(source_x as f64, source_y_taffy as f64),
        ) {
            children_bounds_rect = Some(children_bounds_rect.map_or(bounds, |r| r.union(bounds)));
        }
    }
    let padding_px = 2.0f64;
    let children_bounds = children_bounds_rect.map_or((0.0f32, 0.0f32, 0.0f32, 0.0f32), |r| {
        let x0 = quantize_blur_pos_px((r.x0 - padding_px).max(0.0) as f32);
        let y0 = quantize_blur_pos_px((r.y0 - padding_px).max(0.0) as f32);
        let x1 = quantize_blur_pos_px((r.x1 + padding_px) as f32);
        let y1 = quantize_blur_pos_px((r.y1 + padding_px) as f32);
        (x0, y0, x1 - x0, y1 - y0)
    });

    // Store the source rectangle
    // On macOS/iOS: source_y_taffy is Y-down from top, so we store it directly
    // The copy code will handle platform-specific Y coordinate conversion
    log::debug!(
        "[Blur] view_id={} source_rect=({:.1},{:.1}) size={:.1}x{:.1} parent_bg_check: y={:.1} h={:.1}",
        id,
        source_x,
        source_y_taffy,
        node_width,
        node_height,
        local_transform.as_coeffs()[5] - node_height,
        node_height
    );

    if let Some(index) = existing_index {
        // Update existing entry's metadata but reuse the texture
        let entry = &mut blurred_textures[index];
        entry.last_seen_frame = blur_scene_frame;

        // ── Per-entry dirty detection ──
        let radius_value_changed = (entry.prev_blur_radius - blur.blur_radius).abs() > 0.25;
        let style_changed = blur.blur_style != entry.blur_style;
        // The current Dual-Kawase implementation only changes the actual blur
        // kernel when the pass class changes. Radius changes inside the same
        // pass class are visually identical as long as the padded texture size
        // is unchanged, so do not spend the Android budget on a no-op re-blur.
        let blur_kernel_changed = radius_value_changed
            && kawase_pass_class_for_radius(entry.prev_blur_radius)
                != kawase_pass_class_for_radius(blur.blur_radius);
        let new_source_rect = (
            source_x,
            source_y_taffy,
            quantize_blur_size_px(node_width as f32),
            quantize_blur_size_px(node_height as f32),
        );
        let rect_changed = blur_rect_changed(entry.prev_source_rect, new_source_rect);
        let mut param_dirty_bits = 0u32;
        if blur_kernel_changed {
            param_dirty_bits |= PARAM_DIRTY_RADIUS;
        }
        if style_changed {
            param_dirty_bits |= PARAM_DIRTY_STYLE;
        }
        if (entry.prev_source_rect.0 - new_source_rect.0).abs() >= BLUR_SOURCE_RECT_EPS_PX {
            param_dirty_bits |= PARAM_DIRTY_SRC_X;
        }
        if (entry.prev_source_rect.1 - new_source_rect.1).abs() >= BLUR_SOURCE_RECT_EPS_PX {
            param_dirty_bits |= PARAM_DIRTY_SRC_Y;
        }
        if (entry.prev_source_rect.2 - new_source_rect.2).abs() >= BLUR_SOURCE_RECT_EPS_PX {
            param_dirty_bits |= PARAM_DIRTY_SRC_W;
        }
        if (entry.prev_source_rect.3 - new_source_rect.3).abs() >= BLUR_SOURCE_RECT_EPS_PX {
            param_dirty_bits |= PARAM_DIRTY_SRC_H;
        }
        let active_size_changed = entry.width != texture_width || entry.height != texture_height;
        let allocation_changed =
            entry.allocated_width < texture_width || entry.allocated_height < texture_height;
        let opacity_changed = (entry.prev_opacity - blur.opacity).abs() > f32::EPSILON;
        let overlay_changed = entry.prev_overlay_color != blur.overlay_color;
        let children_bounds_changed = (entry.children_bounds.0 - children_bounds.0).abs()
            >= BLUR_SOURCE_RECT_EPS_PX
            || (entry.children_bounds.1 - children_bounds.1).abs() >= BLUR_SOURCE_RECT_EPS_PX
            || (entry.children_bounds.2 - children_bounds.2).abs() >= BLUR_SOURCE_RECT_EPS_PX
            || (entry.children_bounds.3 - children_bounds.3).abs() >= BLUR_SOURCE_RECT_EPS_PX;
        let children_size_changed = (entry.children_bounds.2 - children_bounds.2).abs()
            >= BLUR_SOURCE_RECT_EPS_PX
            || (entry.children_bounds.3 - children_bounds.3).abs() >= BLUR_SOURCE_RECT_EPS_PX;
        let children_changed = entry.deferred_children != deferred_children || children_bounds_changed;

        let computed_dirty_kind = if allocation_changed {
            // Texture recreation needed — full redo. This should be rare now
            // because backing textures grow in buckets instead of matching
            // every active size exactly.
            BlurDirtyKind::BackgroundChanged
        } else if blur_kernel_changed || rect_changed || active_size_changed {
            BlurDirtyKind::BlurParamsChanged
        } else if opacity_changed || overlay_changed || style_changed {
            BlurDirtyKind::OverlayOnlyChanged
        } else if children_changed {
            BlurDirtyKind::ChildrenChanged
        } else {
            BlurDirtyKind::Clean
        };
        entry.dirty_kind = if entry.blur_rebuild_pending
            && matches!(
                computed_dirty_kind,
                BlurDirtyKind::Clean | BlurDirtyKind::OverlayOnlyChanged
            )
        {
            BlurDirtyKind::BlurParamsChanged
        } else {
            computed_dirty_kind
        };
        entry.param_dirty_bits = if entry.dirty_kind == BlurDirtyKind::BlurParamsChanged {
            param_dirty_bits
        } else {
            0
        };
        if matches!(
            entry.dirty_kind,
            BlurDirtyKind::BackgroundChanged | BlurDirtyKind::BlurParamsChanged
        ) {
            entry.blur_rebuild_pending = true;
        }

        // Update prev_* snapshots for next frame's comparison
        entry.prev_blur_radius = blur.blur_radius;
        entry.prev_source_rect = new_source_rect;
        entry.prev_opacity = blur.opacity;
        entry.prev_overlay_color = blur.overlay_color;

        entry.transform = final_transform;
        entry.opacity = blur.opacity;
        entry.overlay_color = neutral_to_peniko_color(blur.overlay_color);
        entry.border_radius = blur.border_radius as f64;
        entry.source_rect = new_source_rect;
        entry.deferred_children = deferred_children;
        entry.children_bounds = children_bounds;
        entry.backdrop_lod = backdrop_lod;
        if active_size_changed {
            entry.width = texture_width;
            entry.height = texture_height;
            entry.atlas_valid = false;
            entry.atlas_dirty = true;
        }
        // Children texture lifecycle is managed in Pass 3; reset only when bounds change significantly
        if children_size_changed {
            entry.children_texture = None;
            entry.children_texture_view = None;
            entry.children_bind_group = None;
            entry.last_children_uniform_data = None;
            entry.last_children_overlay_data = None;
        }
        entry.blur_radius = blur.blur_radius;
        entry.blur_style = blur.blur_style;
        entry.skipped_due_to_size = false;
        if allocation_changed {
            // Need to recreate texture with new size
            log::debug!(
                "[Blur] Recreating texture for view_id={} due to allocation growth (active {}x{} -> {}x{}, alloc {}x{} -> {}x{})",
                id,
                entry.width,
                entry.height,
                texture_width,
                texture_height,
                entry.allocated_width,
                entry.allocated_height,
                allocated_texture_width,
                allocated_texture_height
            );
            let (tex, view) =
                offscreen_texture.expect("allocation_changed implies needs_new_texture");
            entry.texture = tex;
            entry.texture_view = view;
            entry.width = texture_width;
            entry.height = texture_height;
            entry.allocated_width = allocated_texture_width;
            entry.allocated_height = allocated_texture_height;
            entry.blur_valid = false;
            entry.blur_rebuild_pending = true;
            entry.atlas_valid = false;
            entry.atlas_dirty = true;
            entry.composite_bind_group = None;
            entry.last_composite_uniform_data = None;
            entry.last_composite_overlay_data = None;
            entry.composite_uses_backdrop = false;
        } else {
            log::debug!(
                "[Blur] Reusing cached texture for view_id={} dirty={:?}",
                id,
                entry.dirty_kind
            );
        }
    } else {
        // Create new entry
        let (tex, view) = offscreen_texture.expect("new entry must have texture");
        let new_source_rect = (
            source_x,
            source_y_taffy,
            quantize_blur_size_px(node_width as f32),
            quantize_blur_size_px(node_height as f32),
        );
        blurred_textures.push(BlurredTextureEntry {
            texture: tex,
            texture_view: view,
            width: texture_width,
            height: texture_height,
            allocated_width: allocated_texture_width,
            allocated_height: allocated_texture_height,
            transform: final_transform,
            opacity: blur.opacity,
            overlay_color: neutral_to_peniko_color(blur.overlay_color),
            border_radius: blur.border_radius as f64,
            source_rect: new_source_rect,
            deferred_children,
            children_bounds,
            children_texture: None,
            children_texture_view: None,
            view_id: id,
            blur_radius: blur.blur_radius,
            blur_style: blur.blur_style,
            skipped_due_to_size: false,
            dirty_kind: BlurDirtyKind::BackgroundChanged,
            prev_blur_radius: blur.blur_radius,
            prev_source_rect: new_source_rect,
            prev_opacity: blur.opacity,
            prev_overlay_color: blur.overlay_color,
            param_dirty_bits: 0,
            backdrop_lod,
            last_seen_frame: blur_scene_frame,
            blur_valid: false,
            blur_rebuild_pending: true,
            last_blur_rebuild_frame: 0,
            composite_uniform_buffer: None,
            composite_overlay_buffer: None,
            composite_bind_group: None,
            composite_uses_backdrop: false,
            last_composite_uniform_data: None,
            last_composite_overlay_data: None,
            children_uniform_buffer: None,
            children_overlay_buffer: None,
            children_bind_group: None,
            last_children_uniform_data: None,
            last_children_overlay_data: None,
            atlas_valid: false,
            atlas_dirty: true,
            atlas_x: 0,
            atlas_y: 0,
        });
    }

    // NOTE: For proper frosted glass effect, we do NOT draw the node's background
    // to the main scene. Instead, we want to blur the content BEHIND the node.
    // The blurred background will be composited later with a translucent tint.
    //
    // This ensures the frosted glass shows the blurred background, not its own color.

    // Children are deferred - don't render them here
    // They will be rendered after the blurred background is composited

    true
}

#[allow(dead_code)]
/// Helper to render a child node to the blur temp scene
fn render_child_to_blur_scene(
    id: u32,
    nodes: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
    scene: &mut Scene,
    transform: Affine,
    padding_offset: f64,
) {
    use kurbo::{Rect as KRect, RoundedRect};
    use vello::peniko::Fill;

    if let Some(node) = nodes.get(&id).copied() {
        let x = node.x as f64 + node.position_x as f64 + padding_offset;
        let y = node.y as f64 + node.position_y as f64 + padding_offset;
        let width = node.width as f64;
        let height = node.height as f64;

        let local_transform = transform * Affine::translate((x, y));

        // Draw the child
        let rect = KRect::from_origin_size((0.0, 0.0), (width, height));
        let color = match node.content {
            dyxel_render_api::NodeContent::Rect { color } => neutral_to_peniko_color(color),
            _ => peniko::Color::new([0.0, 0.0, 0.0, 0.0]),
        };
        if node.border_radius > 0.0 {
            let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
            scene.fill(Fill::NonZero, local_transform, color, None, &rounded);
        } else {
            scene.fill(Fill::NonZero, local_transform, color, None, &rect);
        }

        // Recursively render grandchildren
        for &child_id in &node.children {
            render_child_to_blur_scene(child_id, nodes, scene, local_transform, 0.0);
        }
    }
}

/// Compute the axis-aligned bounding box of a subtree in screen coordinates.
/// Returns None if the node does not exist.
fn compute_subtree_bounds(
    id: u32,
    nodes: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
    parent_pos: Vec2,
) -> Option<kurbo::Rect> {
    let node = nodes.get(&id).copied()?;
    let x = node.x as f64 + node.position_x as f64;
    let y = node.y as f64 + node.position_y as f64;
    let width = node.width as f64;
    let height = node.height as f64;

    let global_x = parent_pos.x + x;
    let global_y = parent_pos.y + y;
    let mut bounds = kurbo::Rect::from_origin_size((global_x, global_y), (width, height));

    let child_pos = parent_pos + Vec2::new(x, y);
    for &child_id in &node.children {
        if let Some(child_bounds) = compute_subtree_bounds(child_id, nodes, child_pos) {
            bounds = bounds.union(child_bounds);
        }
    }

    Some(bounds)
}

/// Render a deferred child (for frosted glass effect)
/// This renders children of blur views on top of the blurred background
fn render_deferred_child(
    id: u32,
    nodes: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
    scene: &mut Scene,
    parent_pos: Vec2,
    origin_offset: Vec2,
    glyph_run_cache: &SharedMutex<std::collections::HashMap<GlyphRunCacheKey, GlyphRunCacheEntry>>,
    glyph_run_cache_stats: &SharedMutex<GlyphRunCacheStats>,
) {
    use kurbo::{Rect as KRect, RoundedRect};
    use vello::peniko::{BlendMode as PenikoBlendMode, Compose, Fill, Mix};

    if let Some(node) = nodes.get(&id).copied() {
        let x = node.x as f64 + node.position_x as f64;
        let y = node.y as f64 + node.position_y as f64;
        let width = node.width as f64;
        let height = node.height as f64;

        let local_transform = Affine::translate((parent_pos.x + x - origin_offset.x, parent_pos.y + y - origin_offset.y));

        // Apply opacity using layer if needed
        let needs_layer = node.opacity < 1.0;
        if needs_layer {
            let alpha = node.opacity.clamp(0.0, 1.0);
            let blend = PenikoBlendMode::new(Mix::Normal, Compose::SrcOver);
            let rect = KRect::from_origin_size((0.0, 0.0), (width, height));
            scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &rect);
        }

        // Draw the child
        if let dyxel_render_api::NodeContent::Text(ref payload) = node.content {
            draw_prepared_text(scene, payload, local_transform, glyph_run_cache, glyph_run_cache_stats, 1.0);
        } else if let dyxel_render_api::NodeContent::Rect { color } = node.content {
            let rect = KRect::from_origin_size((0.0, 0.0), (width, height));
            let pcolor = neutral_to_peniko_color(color);
            if node.border_radius > 0.0 {
                let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                scene.fill(Fill::NonZero, local_transform, pcolor, None, &rounded);
            } else {
                scene.fill(Fill::NonZero, local_transform, pcolor, None, &rect);
            }
        }

        // Pop layer if pushed
        if needs_layer {
            scene.pop_layer();
        }

        // Recursively render grandchildren
        let child_pos = parent_pos + Vec2::new(x, y);
        for &child_id in &node.children {
            render_deferred_child(child_id, nodes, scene, child_pos, origin_offset, glyph_run_cache, glyph_run_cache_stats);
        }
    }
}

/// Draw prepared text payload into the scene.
/// Consumes PreparedText directly: decorations (selection/cursor) + glyph runs.
/// Uses GlyphRunCache to avoid re-mapping glyphs every frame for static text.
fn draw_prepared_text(
    scene: &mut Scene,
    payload: &dyxel_render_api::TextDrawPayload,
    transform: Affine,
    glyph_run_cache: &SharedMutex<std::collections::HashMap<GlyphRunCacheKey, GlyphRunCacheEntry>>,
    glyph_run_cache_stats: &SharedMutex<GlyphRunCacheStats>,
    opacity: f32,
) {
    use peniko::{Brush, Fill};
    use std::hash::{Hash, Hasher};

    // 1. Draw decorations (selection background, cursor)
    for deco in &payload.prepared.decorations {
        let rect = kurbo::Rect::new(
            deco.x as f64,
            deco.y as f64,
            (deco.x + deco.width) as f64,
            (deco.y + deco.height) as f64,
        );
        scene.fill(
            Fill::NonZero,
            transform,
            neutral_to_peniko_color(apply_opacity_to_color(deco.color, opacity)),
            None,
            &rect,
        );
    }

    // 2. Draw glyph runs (with cache)
    let current_frame = FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed);
    for run in &payload.prepared.glyph_runs {
        let font_data = run.font_data.downcast_ref::<peniko::FontData>()
            .expect("font_data is not peniko::FontData");
        let font_id = font_data.data.id();
        let font_size_quanted = (run.font_size * 2.0) as u32;

        // Compute glyph signature: fxhash of glyph ids only
        // (x/y change with position, but glyph ids identify the text content)
        let mut hasher = rustc_hash::FxHasher::default();
        for g in &run.glyphs {
            g.id.hash(&mut hasher);
        }
        let glyph_signature = hasher.finish();
        let effective_color = apply_opacity_to_color(run.color, opacity);

        let cache_key = GlyphRunCacheKey {
            font_ptr: font_id as usize,
            font_size_quanted,
            color: effective_color,
            glyph_signature,
        };

        let mut cache = glyph_run_cache.lock().unwrap();
        if let Some(entry) = cache.get_mut(&cache_key) {
            // Cache hit: reuse pre-built glyphs
            entry.last_used_frame.store(
                current_frame,
                std::sync::atomic::Ordering::Relaxed,
            );
            let glyphs = entry.glyphs.iter().cloned();
            scene
                .draw_glyphs(font_data)
                .brush(Brush::Solid(neutral_to_peniko_color(effective_color)))
                .hint(true)
                .transform(transform)
                .font_size(run.font_size)
                .draw(Fill::NonZero, glyphs);
            drop(cache);
            glyph_run_cache_stats.lock().unwrap().hits += 1;
        } else {
            drop(cache);
            glyph_run_cache_stats.lock().unwrap().misses += 1;

            // Cache miss: build glyphs
            let glyphs: Vec<vello::Glyph> = run.glyphs.iter().map(|g| vello::Glyph {
                id: g.id,
                x: g.x,
                y: g.y,
            }).collect();

            scene
                .draw_glyphs(font_data)
                .brush(Brush::Solid(neutral_to_peniko_color(effective_color)))
                .hint(true)
                .transform(transform)
                .font_size(run.font_size)
                .draw(Fill::NonZero, glyphs.iter().cloned());

            // Only cache if under size limit to prevent HashMap bloat
            let mut cache = glyph_run_cache.lock().unwrap();
            if cache.len() < 1000 {
                let entry = GlyphRunCacheEntry {
                    glyphs,
                    last_used_frame: AtomicU64::new(current_frame),
                };
                cache.insert(cache_key, entry);
            }
        }
    }
}

impl VelloBackend {
    /// Render a node with layer effects (alpha, blur, shadow, clip)
    /// Following Xilem's pattern: shadow -> content -> children
    fn render_node_recursive_internal(
        &self,
        id: u32,
        nodes: &std::collections::HashMap<u32, &dyxel_render_api::SceneNode>,
        scene: &mut Scene,
        parent_pos: Vec2,
        transform: Affine,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut vello::Renderer,
        filter_pipeline: Option<&crate::filter_pipeline::FilterPipeline>,
        blurred_textures: &mut Vec<BlurredTextureEntry>,
        cached_textures: &std::collections::HashMap<u32, dyxel_render_api::raster_cache::TextureId>,
        cached_draws: &mut Vec<CachedDraw>,
        in_blur_subtree: bool,
        blur_scene_frame: u64,
    ) {
    use kurbo::{Affine, Rect as KRect, RoundedRect};
    use vello::peniko::{BlendMode as PenikoBlendMode, Compose, Fill, Mix};


    if let Some(node) = nodes.get(&id).copied() {
        let taffy_x = node.x as f64;
        let taffy_y = node.y as f64;
        let node_width = node.width as f64;
        let node_height = node.height as f64;
        let pos_offset = Vec2::new(node.position_x as f64, node.position_y as f64);

        // When position is set, treat it as absolute coordinates within the parent
        // (ignoring Taffy layout position) rather than an offset on top of layout.
        let is_absolute = node.position_x != 0.0 || node.position_y != 0.0;
        let global_pos = if is_absolute {
            parent_pos + pos_offset
        } else {
            parent_pos + Vec2::new(taffy_x, taffy_y)
        };

        // Build local transform for this node
        let local_transform = transform * Affine::translate((global_pos.x, global_pos.y));

        // Determine if we need layer effects
        let _has_shadow = node.shadow.is_some();
        let has_blur = node.blur.is_some();
        let has_children = !node.children.is_empty();
        // OPTIMIZATION: Leaf nodes with only opacity don't need a layer.
        // We can apply opacity directly to the fill color, avoiding costly
        // per-tile clip commands that blow up the PTCL buffer.
        let needs_layer_for_opacity = node.opacity < 1.0 && has_children;
        let needs_layer = needs_layer_for_opacity || node.clip_to_bounds || has_blur;

        // === Raster Cache Check ===
        // Conservative eligibility: only nodes fully outside any blur subtree.
        // Backend performs read-only lookup; Runtime decides which nodes to bake.
        let node_in_blur_subtree = in_blur_subtree || has_blur;
        if !node_in_blur_subtree {
            if let Some(&texture_id) = cached_textures.get(&id) {
                cached_draws.push(CachedDraw {
                    texture_id: texture_pool::TextureId(texture_id.0),
                    transform: Affine::translate((global_pos.x, global_pos.y)),
                    width: node_width as f32,
                    height: node_height as f32,
                });
                return;
            }
        }

        // NOTE: When blur is enabled, we skip layer creation here because:
        // 1. The node's background should NOT be drawn to the main scene
        // 2. Blur effect handles opacity and compositing separately
        let needs_layer_without_blur = needs_layer && !has_blur;

        // Debug: Log blur node info
        if has_blur {
            log::debug!(
                "[Debug] Blur node id={} blur_radius={} opacity={}",
                id,
                node.blur.as_ref().map(|b| b.blur_radius).unwrap_or(0.0),
                node.opacity
            );
            log::debug!(
                "[Debug] Position: taffy=({:.1},{:.1}) global=({:.1},{:.1}) size={:.1}x{:.1}",
                taffy_x,
                taffy_y,
                global_pos.x,
                global_pos.y,
                node_width,
                node_height
            );
            log::debug!(
                "[Debug] BEFORE check: id={} needs_layer={} has_blur={} needs_layer_without_blur={}",
                id,
                needs_layer,
                has_blur,
                needs_layer_without_blur
            );
        }

        // === Step 1: Draw Shadow (if any, using blur) ===
        // Xilem pattern: Draw shadow first, then content on top
        // NOTE: When blur is enabled, skip shadow in Pass 1. Shadow will be handled
        // by the blur compositing pipeline to avoid double-rendering.
        log::debug!("[ShadowCheck] id={} has_shadow={} has_blur={}", id, node.shadow.is_some(), has_blur);
        if let Some(ref shadow) = node.shadow {
            if !has_blur {
                log::debug!("[ShadowDraw] id={} offset=({},{}) blur={} color={:?}", id, shadow.offset_x, shadow.offset_y, shadow.blur, shadow.color);
                let shadow_x = shadow.offset_x as f64;
                let shadow_y = shadow.offset_y as f64;
                let blur_radius = shadow.blur as f64;
                let shadow_color = neutral_to_peniko_color(shadow.color);

                // Try shadow cache first
                let cache_key = ShadowCacheKey {
                    width: node_width as u16,
                    height: node_height as u16,
                    border_radius: (node.border_radius * 2.0) as u16,
                    blur_radius: (shadow.blur * 2.0) as u16,
                    color: shadow.color,
                };

                let mut cache_guard = self.shadow_cache.lock().unwrap();
                if let Some(entry) = cache_guard.get_mut(&cache_key) {
                    // Cache hit: draw cached shadow texture
                    entry.last_used_frame.store(
                        FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    let brush = peniko::ImageBrush::new(entry.image_data.clone())
                        .with_alpha(shadow_color.components[3]);
                    let blur_pad = blur_radius * 2.0;
                    let image_transform = local_transform
                        * Affine::translate((shadow_x - blur_pad, shadow_y - blur_pad));
                    scene.draw_image(&brush, image_transform);
                    drop(cache_guard);
                    self.shadow_cache_stats.lock().unwrap().hits += 1;
                } else {
                    drop(cache_guard);
                    self.shadow_cache_stats.lock().unwrap().misses += 1;

                    // Cap per-frame cache misses to avoid GPU submit spikes.
                    // On cold start shadows warm up over ~30 frames instead of one.
                    let misses = self
                        .shadow_cache_misses_this_frame
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if misses >= 5 {
                        // Fallback: draw shadow directly without caching this frame
                        let shadow_rect = KRect::from_origin_size(
                            (shadow_x, shadow_y),
                            (node_width, node_height),
                        );
                        scene.draw_blurred_rounded_rect(
                            local_transform,
                            shadow_rect,
                            shadow_color,
                            node.border_radius as f64,
                            blur_radius,
                        );
                    } else {
                        // Cache miss: render shadow to a new texture and cache it
                        let blur_pad = blur_radius * 2.0;
                        let tex_w = (node_width + blur_pad * 2.0).ceil() as u32;
                        let tex_h = (node_height + blur_pad * 2.0).ceil() as u32;

                        let texture = device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("shadow_cache_texture"),
                            size: wgpu::Extent3d {
                                width: tex_w.max(1),
                                height: tex_h.max(1),
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::STORAGE_BINDING
                                | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        });

                        let mut shadow_scene = Scene::new();
                        let rect = KRect::from_origin_size(
                            (blur_pad, blur_pad),
                            (node_width, node_height),
                        );
                        if node.border_radius > 0.0 {
                            shadow_scene.draw_blurred_rounded_rect(
                                Affine::IDENTITY,
                                rect,
                                shadow_color,
                                node.border_radius as f64,
                                blur_radius,
                            );
                        } else {
                            shadow_scene.draw_blurred_rounded_rect(
                                Affine::IDENTITY,
                                rect,
                                shadow_color,
                                0.0,
                                blur_radius,
                            );
                        }

                        let texture_view =
                            texture.create_view(&wgpu::TextureViewDescriptor::default());
                        match renderer.render_to_texture(
                            device,
                            queue,
                            &shadow_scene,
                            &texture_view,
                            &vello::RenderParams {
                                base_color: peniko::Color::new([0.0, 0.0, 0.0, 0.0]),
                                width: tex_w,
                                height: tex_h,
                                antialiasing_method: vello::AaConfig::Area,
                            },
                        ) {
                            Ok(()) => {
                                let image_data = renderer.register_texture(texture);

                                // Draw the newly cached shadow in the current frame
                                let brush = peniko::ImageBrush::new(image_data.clone())
                                    .with_alpha(shadow_color.components[3]);
                                let image_transform = local_transform
                                    * Affine::translate((shadow_x - blur_pad, shadow_y - blur_pad));
                                scene.draw_image(&brush, image_transform);

                                let entry = ShadowCacheEntry {
                                    image_data,
                                    last_used_frame: AtomicU64::new(
                                        FRAME_COUNTER.load(std::sync::atomic::Ordering::Relaxed),
                                    ),
                                };
                                self.shadow_cache.lock().unwrap().insert(cache_key, entry);
                            }
                            Err(e) => {
                                log::warn!("[ShadowCache] render_to_texture failed: {:?}. Falling back to direct draw.", e);
                                // Fallback: draw shadow directly
                                let shadow_rect = KRect::from_origin_size(
                                    (shadow_x, shadow_y),
                                    (node_width, node_height),
                                );
                                scene.draw_blurred_rounded_rect(
                                    local_transform,
                                    shadow_rect,
                                    shadow_color,
                                    node.border_radius as f64,
                                    blur_radius,
                                );
                            }
                        }
                    }
                }
            }
        }

        // === Step 2: Push Layer (if needed for alpha/blur/clip) ===
        // NOTE: When blur is enabled, we skip layer creation here because:
        // 1. The node's background should NOT be drawn to the main scene
        // 2. Blur effect handles opacity and compositing separately

        log::debug!("[LayerCheck] id={} needs_layer={} clip_to_bounds={} opacity={} border_radius={}", id, needs_layer_without_blur, node.clip_to_bounds, node.opacity, node.border_radius);
        if needs_layer_without_blur {
            // Convert opacity to layer alpha
            let alpha = node.opacity.clamp(0.0, 1.0);

            // Default blend mode (Normal)
            let blend = PenikoBlendMode::new(Mix::Normal, Compose::SrcOver);

            // Use node's bounds for the layer shape to avoid full-screen clip bloat.
            // If clip_to_bounds is enabled, we clip exactly to the node bounds.
            // Otherwise we still use node bounds (not infinite rect) for performance.
            if node.border_radius > 0.0 {
                let clip_rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
                let rounded_clip = RoundedRect::from_rect(clip_rect, node.border_radius as f64);
                scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &rounded_clip);
            } else {
                let clip_rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
                scene.push_layer(Fill::NonZero, blend, alpha, local_transform, &clip_rect);
            }
        }

        // === Step 3: Handle Blur Effect ===
        // If blur is enabled, render to offscreen texture and apply blur
        let blur_applied = if let Some(ref blur) = node.blur {
            if filter_pipeline.is_some() {
                render_with_blur(
                    blur,
                    id,
                    nodes,
                    scene,
                    local_transform,
                    device,
                    queue,
                    renderer,
                    filter_pipeline.unwrap(),
                    node_width,
                    node_height,
                    needs_layer,
                    blurred_textures,
                    blur_scene_frame,
                )
            } else {
                false
            }
        } else {
            false
        };

        // === Step 4: Draw Node Content ===
        // Skip normal drawing if blur was applied (blur texture will be drawn in blit pass)
        // Opacity is applied either by the layer (Step 2) or baked into content color here.
        // If a layer was pushed for opacity, we must NOT double-apply it to content.
        let direct_opacity = if needs_layer_without_blur { 1.0 } else { node.opacity };
        if !blur_applied {
            match node.content {
                dyxel_render_api::NodeContent::Text(ref payload) => {
                    draw_prepared_text(scene, payload, local_transform, &self.glyph_run_cache, &self.glyph_run_cache_stats, direct_opacity);
                }
                dyxel_render_api::NodeContent::Rect { color } => {
                    let rect = KRect::from_origin_size((0.0, 0.0), (node_width, node_height));
                    // Apply opacity directly only when no layer is handling it
                    let effective_color = if direct_opacity < 1.0 {
                        apply_opacity_to_color(color, direct_opacity)
                    } else {
                        color
                    };
                    let pcolor = neutral_to_peniko_color(effective_color);

                    // Debug: Log fill operations for non-text nodes
                    log::debug!(
                        "[DebugFill] id={} color={:?} size={}x{} transform={:?}",
                        id,
                        color,
                        node_width,
                        node_height,
                        local_transform
                    );

                    if node.border_radius > 0.0 {
                        let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
                        scene.fill(Fill::NonZero, local_transform, pcolor, None, &rounded);
                    } else {
                        scene.fill(Fill::NonZero, local_transform, pcolor, None, &rect);
                    }
                }
            }
        }

        // === Step 5: Recursively render children ===
        // For blur views: skip children in Pass 1, they will be rendered to
        // a separate texture in Pass 3 and composited on top of blur in blit pass.
        // For non-blur views: render children normally.
        // DEBUG: Log children traversal
        if !node.children.is_empty() {
            log::debug!(
                "[DebugChildren] id={} has {} children: {:?}",
                id,
                node.children.len(),
                node.children
            );
        }
        if !blur_applied {
            for &child_id in &node.children {
                self.render_node_recursive_internal(
                    child_id,
                    nodes,
                    scene,
                    global_pos,
                    transform,
                    device,
                    queue,
                    renderer,
                    filter_pipeline,
                    blurred_textures,
                    cached_textures,
                    cached_draws,
                    node_in_blur_subtree,
                    blur_scene_frame,
                );
            }
        }

        // === Step 6: Pop Layer (if pushed) ===
        // Only pop layer if we pushed it (when blur is NOT enabled)
        if needs_layer_without_blur {
            scene.pop_layer();
        }
    }
}
}

impl RenderBackend for VelloBackend {
    fn init(
        &self,
        device: DeviceHandle,
        _queue: QueueHandle,
        config: BackendConfig,
    ) -> RenderResult {
        let init_start = std::time::Instant::now();

        #[cfg(target_os = "android")]
        log::info!("[Android-Perf] VelloBackend::init started - Performance monitoring enabled");

        // Convert DeviceHandle to wgpu::Device reference
        let device = unsafe { &*device.as_ptr::<wgpu::Device>() };

        // Try using pre-compiled SPIR-V, fall back to WGSL if it fails
        let blit_shader = if cfg!(target_os = "android") {
            let spv_words: Vec<u32> = BLIT_SHADER_SPV
                .chunks_exact(4)
                .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();

            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blit Shader (SPIR-V)"),
                source: wgpu::ShaderSource::SpirV(std::borrow::Cow::Owned(spv_words)),
            })
        } else {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blit Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
            })
        };

        let blit_bl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let cache_path = format!("{}/vello_v1.cache", config.data_dir);
        log::info!("[ColdStart] Pipeline cache path: {}", cache_path);

        // Detailed cache loading diagnostics with Stage detection
        #[cfg(not(target_arch = "wasm32"))]
        let (cache_stage, cache_data) = match std::fs::read(&cache_path) {
            Ok(data) if data.len() > 1 => {
                // Check for stage marker (first byte)
                let stage = data[0];
                let actual_data = &data[1..];

                match stage {
                    1 => log::info!(
                        "[ColdStart] Stage 1 cache loaded: {} bytes (area_only)",
                        actual_data.len()
                    ),
                    2 => log::info!(
                        "[ColdStart] Stage 2 cache loaded: {} bytes (full)",
                        actual_data.len()
                    ),
                    _ => log::info!("[ColdStart] Legacy cache loaded: {} bytes", data.len()),
                }

                if stage == 1 || stage == 2 {
                    (Some(stage), Some(actual_data.to_vec()))
                } else {
                    // Legacy cache without marker
                    (None, Some(data))
                }
            }
            Ok(_) => {
                log::info!("[ColdStart] Cache file too small, treating as empty");
                (None, None)
            }
            Err(e) => {
                log::warn!(
                    "[ColdStart] Cache file not loaded: {} (path: {})",
                    e,
                    cache_path
                );
                (None, None)
            }
        };
        #[cfg(target_arch = "wasm32")]
        let (cache_stage, cache_data): (Option<u8>, Option<Vec<u8>>) = (None, None);

        let pipeline_cache_supported = device.features().contains(wgpu::Features::PIPELINE_CACHE);
        log::info!(
            "[ColdStart] PIPELINE_CACHE feature supported: {}",
            pipeline_cache_supported
        );

        let pipeline_cache = if pipeline_cache_supported {
            let start = std::time::Instant::now();
            let cache = Some(unsafe {
                device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("Vello Pipeline Cache"),
                    data: cache_data.as_deref(),
                    fallback: true,
                })
            });
            log::info!(
                "[ColdStart] Pipeline cache creation took: {:?}",
                start.elapsed()
            );
            cache
        } else {
            log::warn!("[ColdStart] PIPELINE_CACHE not supported, skipping cache");
            None
        };

        *self.blit_bind_group_layout.lock().unwrap() = Some(blit_bl);
        *self.sampler.lock().unwrap() = Some(sampler);
        *self.blit_shader.lock().unwrap() = Some(blit_shader);
        *self.pipeline_cache.lock().unwrap() = pipeline_cache.clone();
        *self.cache_path.lock().unwrap() = Some(cache_path.clone());
        *self.cache_stage.lock().unwrap() = cache_stage;

        // Prewarm blit pipeline
        self.prewarm_pipelines(device, wgpu::TextureFormat::Rgba8Unorm);

        // Initialize filter pipeline for blur effects
        let device_arc = std::sync::Arc::new(device.clone());
        let queue_arc = std::sync::Arc::new(unsafe { &*_queue.as_ptr::<wgpu::Queue>() }.clone());
        match filter_pipeline::FilterPipeline::new(device_arc, queue_arc) {
            Ok(pipeline) => {
                *self.filter_pipeline.lock().unwrap() = Some(pipeline);
                log::debug!("[Blur] Filter pipeline initialized successfully");
            }
            Err(e) => {
                log::warn!("[Blur] Failed to initialize filter pipeline: {}", e);
                // Continue without blur support
            }
        }

        // Note: Blur composite pipeline is created lazily on first use
        // with the correct surface format to avoid format mismatch

        // Initialize texture pool for efficient blur texture reuse
        {
            let device_arc = Arc::new(device.clone());
            let pool = texture_pool::SharedTexturePool::new(
                device_arc.clone(),
                texture_pool::TexturePoolConfig::default(),
            );
            *self.texture_pool.lock().unwrap() = Some(pool);
            let gpu_pool = texture_pool::GpuTexturePool::new(
                device_arc,
                texture_pool::TexturePoolConfig::default(),
            );
            *self.gpu_texture_pool.lock().unwrap() = Some(gpu_pool);
            log::info!("[TexturePool] Initialized blur texture pool");
        }

        // Raster cache initialization has moved to Runtime.

        // Store info for deferred renderer initialization (includes cache stage)
        *self.init_device_info.lock().unwrap() = Some((cache_path, pipeline_cache, cache_stage));

        // Eagerly start renderer initialization in background so first frame isn't black
        let queue_ref = unsafe { &*_queue.as_ptr::<wgpu::Queue>() };
        self.ensure_renderer_initialized_async(device, queue_ref);

        // Initialize memory optimizer
        {
            let memory_optimizer = self.memory_optimizer.lock().unwrap();
            memory_optimizer.initialize();
            log::info!(
                "[Memory] Initialized memory optimizer for tier: {:?}",
                memory_optimizer.tier()
            );
        }

        log::info!(
            "[Perf] VelloBackend::init: Total time {:?} (Renderer deferred)",
            init_start.elapsed()
        );
        Ok(())
    }

    fn create_surface_state(
        &self,
        context: &mut RenderContext,
        target: Option<SurfaceTargetHandle>,
        surface: Option<SurfaceHandle>,
        _surface_ptr: u64,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Box<dyn SurfaceState>> {
        log::info!(
            "VelloBackend: create_surface_state START - size: {}x{}, has_precreated_surface: {}",
            width,
            height,
            surface.is_some()
        );

        // Downcast RenderContext to vello::util::RenderContext
        let v_ctx = context
            .downcast_mut::<vello::util::RenderContext>()
            .ok_or_else(|| anyhow::anyhow!("RenderContext is not a Vello RenderContext"))?;

        // Select present mode
        #[cfg(target_os = "android")]
        let present_mode = {
            log::info!("VelloBackend: Using Mailbox mode (low latency, VSync-like but faster)");
            wgpu::PresentMode::Mailbox
        };

        #[cfg(not(target_os = "android"))]
        let present_mode = {
            log::info!("VelloBackend: Using Immediate mode (VSync disabled)");
            wgpu::PresentMode::Immediate
        };

        let v_surface = if let Some(s) = surface {
            log::info!(
                "VelloBackend: Using pre-created surface (present_mode: {:?})",
                present_mode
            );
            let wgpu_surface = s
                .into_inner::<wgpu::Surface<'static>>()
                .ok_or_else(|| anyhow::anyhow!("SurfaceHandle is not a wgpu::Surface"))?;
            pollster::block_on(v_ctx.create_render_surface(
                wgpu_surface,
                width,
                height,
                present_mode,
            ))
            .map_err(|e| anyhow::anyhow!("Failed to create render surface: {:?}", e))?
        } else if let Some(t) = target {
            log::info!(
                "VelloBackend: Creating surface from target (present_mode: {:?})",
                present_mode
            );
            let wgpu_target = t
                .into_inner::<wgpu::SurfaceTarget<'static>>()
                .ok_or_else(|| {
                    anyhow::anyhow!("SurfaceTargetHandle is not a wgpu::SurfaceTarget")
                })?;
            pollster::block_on(v_ctx.create_surface(wgpu_target, width, height, present_mode))
                .map_err(|e| anyhow::anyhow!("Failed to create surface: {:?}", e))?
        } else {
            return Err(anyhow::anyhow!("Either target or surface must be provided"));
        };

        log::info!(
            "VelloBackend: Surface created, format: {:?}, dev_id: {}",
            v_surface.config.format,
            v_surface.dev_id
        );

        let blit_layout_lock = self.blit_bind_group_layout.lock().unwrap();
        let blit_shader_lock = self.blit_shader.lock().unwrap();

        let device = &v_ctx.devices[v_surface.dev_id].device;

        let bl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[blit_layout_lock.as_ref().unwrap()],
            push_constant_ranges: &[],
        });

        let blit_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&bl),
            vertex: wgpu::VertexState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: v_surface.config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: self.pipeline_cache.lock().unwrap().as_ref(),
        });

        log::info!("VelloBackend: Blit pipeline created successfully");

        // Create children blit pipeline with alpha blending
        let children_blit_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Children Blit Pipeline"),
            layout: Some(&bl),
            vertex: wgpu::VertexState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: v_surface.config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: self.pipeline_cache.lock().unwrap().as_ref(),
        });
        *self.children_blit_pipeline.lock().unwrap() = Some(children_blit_p);
        *self.blit_pipeline.lock().unwrap() = Some(blit_p);

        #[cfg(target_os = "macos")]
        {
            log::info!("VelloBackend: Creating MacVelloSurfaceState");
            return Ok(Box::new(mac::MacVelloSurfaceState {
                surface: v_surface,
            }));
        }

        #[cfg(target_os = "android")]
        {
            log::info!("VelloBackend: Creating AndroidVelloSurfaceState");
            return Ok(Box::new(android::AndroidVelloSurfaceState {
                surface: v_surface,
            }));
        }

        #[cfg(target_arch = "wasm32")]
        {
            log::info!("VelloBackend: Creating WebVelloSurfaceState");
            return Ok(Box::new(web::WebVelloSurfaceState {
                surface: v_surface,
            }));
        }

        #[cfg(all(
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_arch = "wasm32")
        ))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn set_frame_timing(&self, pacer_wait_ms: f64, frame_interval_ms: f64) {
        *self.pacer_wait_ms.lock().unwrap() = pacer_wait_ms;
        *self.frame_interval_ms.lock().unwrap() = frame_interval_ms;
    }

    fn set_frame_performance_stats(&self, stats: dyxel_perf::FramePerformanceStats) {
        *self.frame_perf_stats.lock().unwrap() = stats;
    }

    fn render_package(
        &self,
        device: DeviceHandle,
        queue: QueueHandle,
        surface: &mut dyn SurfaceState,
        package: &dyxel_render_api::RenderPackage,
    ) -> RenderResult {
        let device = unsafe { &*device.as_ptr::<wgpu::Device>() };
        let queue = unsafe { &*queue.as_ptr::<wgpu::Queue>() };

        #[cfg(target_os = "macos")]
        {
            let v_surface = surface
                .as_any_mut()
                .downcast_mut::<mac::MacVelloSurfaceState>()
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid surface state (not MacVelloSurfaceState)")
                })?;
            let st = v_surface
                .surface
                .surface
                .get_current_texture()
                .map_err(|e| anyhow::anyhow!("Failed to get current texture: {:?}", e))?;
            let target_view = st.texture.create_view(&Default::default());
            let result = self.render_internal_impl(
                device,
                queue,
                &target_view,
                v_surface.surface.format,
                package,
            );
            st.present();
            return result;
        }

        #[cfg(target_os = "android")]
        {
            let v_surface = surface
                .as_any_mut()
                .downcast_mut::<android::AndroidVelloSurfaceState>()
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid surface state (not AndroidVelloSurfaceState)")
                })?;
            let st = v_surface
                .surface
                .surface
                .get_current_texture()
                .map_err(|e| anyhow::anyhow!("Failed to get current texture: {:?}", e))?;
            let target_view = st.texture.create_view(&Default::default());
            let result = self.render_internal_impl(
                device,
                queue,
                &target_view,
                v_surface.surface.format,
                package,
            );
            st.present();
            return result;
        }

        #[cfg(target_arch = "wasm32")]
        {
            let v_surface = surface
                .as_any_mut()
                .downcast_mut::<web::WebVelloSurfaceState>()
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid surface state (not WebVelloSurfaceState)")
                })?;
            let st = v_surface
                .surface
                .surface
                .get_current_texture()
                .map_err(|e| anyhow::anyhow!("Failed to get current texture: {:?}", e))?;
            let target_view = st.texture.create_view(&Default::default());
            let result = self.render_internal_impl(
                device,
                queue,
                &target_view,
                v_surface.surface.format,
                package,
            );
            st.present();
            return result;
        }

        #[cfg(all(
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_arch = "wasm32")
        ))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn on_lifecycle_event(&self, event: LifecycleEvent) {
        match event {
            LifecycleEvent::FirstFrameDone | LifecycleEvent::Shutdown => {
                self.save_cache();
            }
            _ => {}
        }
    }

    fn sync_gpu(&self, _device: DeviceHandle, queue: QueueHandle) {
        let queue = unsafe { &*queue.as_ptr::<wgpu::Queue>() };

        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        queue.on_submitted_work_done(move || {
            let _ = tx.send(());
        });

        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(_) => log::info!("VelloBackend: sync_gpu completed successfully"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                log::warn!("VelloBackend: sync_gpu timed out, GPU may be unresponsive");
            }
            Err(e) => log::error!("VelloBackend: sync_gpu error: {:?}", e),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl RenderBackendExt for VelloBackend {
    fn enable_perf_overlay(&self) {
        self.enable_perf_overlay();
    }

    fn disable_perf_overlay(&self) {
        self.disable_perf_overlay();
    }
}

impl VelloBackendExt for VelloBackend {
    fn vello_renderer(&self) -> Option<&dyn Any> {
        // Return the backend itself as Any, caller can downcast to VelloBackend
        // and access renderer through the public renderer field
        Some(self as &dyn Any)
    }
}

/// Factory for creating VelloBackend instances
pub struct VelloBackendFactory;

impl VelloBackendFactory {
    pub fn new() -> Self {
        Self
    }
}

impl dyxel_render_api::RenderBackendFactory for VelloBackendFactory {
    fn create(&self) -> Box<dyn RenderBackend> {
        Box::new(VelloBackend::new())
    }

    fn name(&self) -> &'static str {
        "vello"
    }
}

impl Default for VelloBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for VelloBackendFactory {
    fn default() -> Self {
        Self::new()
    }
}

// Re-export new double-layer types at crate root for convenience.
pub use backend::VelloDrawingBackend;
pub use factory::VelloGraphicsFactory;
pub use frame_context::WgpuFrameContext;
pub use runtime::WgpuRuntime;

#[cfg(test)]
mod tests {
    use super::*;

    /// Test coordinate transformation for blur sampling
    /// Verifies that source rectangle is calculated correctly for two-pass blur
    #[test]
    fn test_blur_source_rect_calculation() {
        // Simulate a view at position (100, 200) with size 120x120
        let local_transform = Affine::translate((100.0, 200.0));
        let node_width = 120.0;
        let node_height = 120.0;

        // Extract translation (same logic as render_with_blur)
        let mat = local_transform.as_coeffs();
        let source_x = mat[4] as f32; // translation x
        let source_y = mat[5] as f32; // translation y

        assert_eq!(source_x, 100.0);
        assert_eq!(source_y, 200.0);

        // Verify source_rect tuple
        let source_rect = (source_x, source_y, node_width as f32, node_height as f32);
        assert_eq!(source_rect, (100.0, 200.0, 120.0, 120.0));
    }

    /// Test Y-coordinate flipping for wgpu texture copy
    /// Vello uses Y-up, wgpu uses Y-down
    #[test]
    fn test_y_flip_calculation() {
        let screen_height = 800u32;
        let src_y = 200.0f32;
        let src_h = 120.0f32;

        // Flip Y coordinate: Vello Y=0 is bottom, wgpu Y=0 is top
        let flipped_y = (screen_height as f32 - src_y - src_h).max(0.0) as u32;

        // Expected: 800 - 200 - 120 = 480
        assert_eq!(flipped_y, 480);
    }

    /// Test padding calculation for blur bleed
    #[test]
    fn test_blur_padding_calculation() {
        let blur_radius = 10.0f64;
        let padding = (blur_radius * 2.5).ceil() as u32;

        // Expected: 10.0 * 2.5 = 25.0, ceil = 25
        assert_eq!(padding, 25);

        let texture_width = (120.0f64 as u32 + padding * 2).max(1);
        let texture_height = (120.0f64 as u32 + padding * 2).max(1);

        // Expected: 120 + 25*2 = 170
        assert_eq!(texture_width, 170);
        assert_eq!(texture_height, 170);
    }

    /// Test transform adjustment for padding offset
    #[test]
    fn test_transform_with_padding_offset() {
        let local_transform = Affine::translate((100.0, 200.0));
        let blur_radius = 10.0f64;
        let padding = (blur_radius * 2.5).ceil() as u32;

        // Adjust transform to account for padding offset
        let final_transform =
            local_transform * Affine::translate((-(padding as f64), -(padding as f64)));
        let final_mat = final_transform.as_coeffs();

        // Translation should be offset by padding
        assert_eq!(final_mat[4], 100.0 - padding as f64); // x: 100 - 25 = 75
        assert_eq!(final_mat[5], 200.0 - padding as f64); // y: 200 - 25 = 175
    }

    /// Test that padding is consistent between texture creation and transform
    #[test]
    fn test_padding_consistency() {
        let blur_radius = 10.0;
        let node_width = 120.0;
        let node_height = 120.0;

        // Calculate padding (same as render_with_blur)
        let padding = (blur_radius as f64 * 2.5).ceil() as u32;

        // Texture size with padding
        let texture_width = (node_width as u32 + padding * 2).max(1);
        let texture_height = (node_height as u32 + padding * 2).max(1);

        // Verify padding is applied equally on both sides
        let inner_width = texture_width - padding * 2;
        let inner_height = texture_height - padding * 2;

        assert_eq!(inner_width, node_width as u32);
        assert_eq!(inner_height, node_height as u32);
    }

    /// Test frosted glass color extraction
    #[test]
    fn test_frosted_glass_color() {
        // Color from layer_effects_demo.rs: (255u32, 255, 255, 180)
        let color = (255u32, 255, 255, 180);

        // Convert to f32 premultiplied (as done in blur_composite.wgsl)
        let alpha = color.3 as f32 / 255.0;
        let r = (color.0 as f32 / 255.0) * alpha;
        let g = (color.1 as f32 / 255.0) * alpha;
        let b = (color.2 as f32 / 255.0) * alpha;

        // White with 180/255 alpha should have premultiplied values
        assert!((r - 0.705).abs() < 0.01, "R should be ~0.705, got {}", r);
        assert!((g - 0.705).abs() < 0.01, "G should be ~0.705, got {}", g);
        assert!((b - 0.705).abs() < 0.01, "B should be ~0.705, got {}", b);
    }

}
