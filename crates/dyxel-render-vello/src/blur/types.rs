// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Data structures for blur collection, caching, atlasing, and compositing.

use kurbo::Affine;
use vello::wgpu;

/// Granular dirty classification for a blur entry.
///
/// Replaces the binary `needs_recalculation` flag so that Pass 2/3/4 can
/// selectively skip work: an opacity-only change doesn't need a Kawase
/// re-blur, and a blur-radius change doesn't need a scene re-copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlurDirtyKind {
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
pub(crate) struct BlurDirtyStats {
    pub(crate) clean: usize,
    pub(crate) background: usize,
    pub(crate) params: usize,
    pub(crate) overlay: usize,
    pub(crate) children: usize,
    pub(crate) invalid: usize,
    pub(crate) pending: usize,
    pub(crate) skipped: usize,
    pub(crate) visible: usize,
    pub(crate) param_radius: usize,
    pub(crate) param_style: usize,
    pub(crate) param_src_x: usize,
    pub(crate) param_src_y: usize,
    pub(crate) param_src_w: usize,
    pub(crate) param_src_h: usize,
    pub(crate) bg_size: usize,
    pub(crate) children_list: usize,
    pub(crate) children_bounds: usize,
}

/// Entry for a blurred texture to be composited.
#[derive(Debug)]
pub(crate) struct BlurredTextureEntry {
    /// The blurred texture (contains blurred background for frosted glass).
    pub(crate) texture: wgpu::Texture,
    /// Pre-created view of the blurred texture for composite reuse.
    pub(crate) texture_view: wgpu::TextureView,
    /// Active blurred content width, including blur padding.
    pub(crate) width: u32,
    /// Active blurred content height, including blur padding.
    pub(crate) height: u32,
    /// Physical GPU texture width. Kept bucketed and >= width.
    pub(crate) allocated_width: u32,
    /// Physical GPU texture height. Kept bucketed and >= height.
    pub(crate) allocated_height: u32,
    /// Position to draw at (with padding offset already applied).
    pub(crate) transform: Affine,
    /// Opacity of the blurred content.
    pub(crate) opacity: f32,
    /// View color to overlay (for frosted glass effect).
    pub(crate) overlay_color: vello::peniko::Color,
    /// Border radius.
    pub(crate) border_radius: f64,
    /// Source rectangle in scene texture (x, y, width, height) in scene coords.
    pub(crate) source_rect: (f32, f32, f32, f32),
    /// Deferred children to render on top of blurred background.
    pub(crate) deferred_children: Vec<u32>,
    /// Bounding box of deferred children in screen coordinates (x, y, w, h).
    pub(crate) children_bounds: (f32, f32, f32, f32),
    /// Cached children texture for local rendering (Pass 3).
    pub(crate) children_texture: Option<wgpu::Texture>,
    /// Cached view of the children texture.
    pub(crate) children_texture_view: Option<wgpu::TextureView>,
    /// View ID for deferred rendering.
    pub(crate) view_id: u32,
    /// Blur radius.
    pub(crate) blur_radius: f32,
    /// Blur style: 0=Light, 1=Dark, 2=ExtraLight, 3=Prominent.
    pub(crate) blur_style: u8,
    /// Skip this entry when generated blur texture is intentionally disabled.
    pub(crate) skipped_due_to_size: bool,
    /// Per-frame dirty classification — drives selective Pass 2/3/4 execution.
    pub(crate) dirty_kind: BlurDirtyKind,
    /// Previous-frame blur_radius for change detection.
    pub(crate) prev_blur_radius: f32,
    /// Previous-frame source_rect for change detection.
    pub(crate) prev_source_rect: (f32, f32, f32, f32),
    /// Previous-frame opacity for change detection.
    pub(crate) prev_opacity: f32,
    /// Previous-frame overlay_color (RGBA) for change detection.
    pub(crate) prev_overlay_color: [u8; 4],
    /// Bitmask describing why the entry was classified as BlurParamsChanged.
    pub(crate) param_dirty_bits: u32,
    /// Backdrop pyramid LOD level (0=full-res, 1=half, 2=quarter, 3=eighth).
    pub(crate) backdrop_lod: u8,
    /// Scene-build frame marker used to retain entries touched this frame.
    pub(crate) last_seen_frame: u64,
    /// Whether `texture` currently contains a valid blurred backdrop.
    pub(crate) blur_valid: bool,
    /// A valid-but-stale blur that still needs background re-copy/reblur.
    pub(crate) blur_rebuild_pending: bool,
    /// Last frame in which this entry's blur texture was rebuilt.
    pub(crate) last_blur_rebuild_frame: u64,
    /// Cached resources for legacy per-entry compositing.
    pub(crate) composite_uniform_buffer: Option<wgpu::Buffer>,
    pub(crate) composite_overlay_buffer: Option<wgpu::Buffer>,
    pub(crate) composite_bind_group: Option<wgpu::BindGroup>,
    pub(crate) composite_uses_backdrop: bool,
    pub(crate) last_composite_uniform_data: Option<[f32; 12]>,
    pub(crate) last_composite_overlay_data: Option<[f32; 12]>,
    /// Cached resources for children compositing.
    pub(crate) children_uniform_buffer: Option<wgpu::Buffer>,
    pub(crate) children_overlay_buffer: Option<wgpu::Buffer>,
    pub(crate) children_bind_group: Option<wgpu::BindGroup>,
    pub(crate) last_children_uniform_data: Option<[f32; 12]>,
    pub(crate) last_children_overlay_data: Option<[f32; 12]>,
    /// Persistent atlas placement for instanced composite.
    pub(crate) atlas_valid: bool,
    pub(crate) atlas_dirty: bool,
    pub(crate) atlas_x: u32,
    pub(crate) atlas_y: u32,
}

/// Reusable full-screen blurred backdrop texture.
pub(crate) struct BackdropBlurTexture {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) struct BlurAtlasTexture {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct BlurInstance {
    /// Screen-space quad rect including blur padding: x, y, width, height.
    pub(crate) rect: [f32; 4],
    /// Atlas-space rect containing the active blurred texture.
    pub(crate) source_rect: [f32; 4],
    /// Overlay rgba.
    pub(crate) color: [f32; 4],
    /// Border_radius, opacity, dark_mode, unused.
    pub(crate) params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct BlurFrameUniform {
    pub(crate) viewport_size: [f32; 2],
    pub(crate) _pad: [f32; 2],
}

pub(crate) struct BlurAtlasLayout {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) slot: u32,
    pub(crate) gap: u32,
    pub(crate) placements: Vec<(usize, u32, u32)>,
}
