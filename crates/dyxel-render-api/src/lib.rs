// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dyxel Render API - Renderer-agnostic abstraction layer
//!
//! This crate provides a pure abstraction layer for rendering backends.
//! It does NOT depend on any specific rendering library (vello, wgpu, etc.).
//!
//! Concrete implementations are provided by separate crates:
//! - dyxel-render-vello: Vello + wgpu implementation
//! - dyxel-render-impeller: Impeller implementation (future)

use std::any::Any;

pub mod dirty;
pub mod filters;
pub mod raster_cache;

// Re-export commonly-used types at crate root for convenience.
pub use dirty::{DirtyField, DirtyTracker};
pub use filters::{BlendMode, Filter, FilterId, FilterType, LayerAttribute, Rect};
pub use raster_cache::TextureId;

/// Callback type for marking nodes as dirty after layout computation
/// Render backend calls this after compute_layout to notify core
pub type LayoutDirtyCallback = Box<dyn Fn(&[u32]) + Send + Sync>;

// Platform-specific types
#[cfg(not(target_arch = "wasm32"))]
pub type SharedPtr<T> = std::sync::Arc<T>;
#[cfg(target_arch = "wasm32")]
pub type SharedPtr<T> = std::rc::Rc<T>;

#[cfg(not(target_arch = "wasm32"))]
pub type SharedMutex<T> = std::sync::Mutex<T>;
#[cfg(target_arch = "wasm32")]
pub type SharedMutex<T> = std::cell::RefCell<T>;

// Helper trait for unified lock API
#[cfg(target_arch = "wasm32")]
pub trait LockExt<T> {
    fn lock(&self) -> Result<std::cell::RefMut<'_, T>, ()>;
}
#[cfg(target_arch = "wasm32")]
impl<T> LockExt<T> for std::cell::RefCell<T> {
    fn lock(&self) -> Result<std::cell::RefMut<'_, T>, ()> {
        Ok(self.borrow_mut())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub trait LockExt<T> {}
#[cfg(not(target_arch = "wasm32"))]
impl<T> LockExt<T> for std::sync::Mutex<T> {}

/// Opaque handle to a GPU device
///
/// This is an opaque pointer that concrete backends use to store their device type.
/// For Vello backend, this points to a `wgpu::Device`.
#[derive(Clone, Copy)]
pub struct DeviceHandle {
    pub(crate) ptr: *const (),
    pub(crate) _marker: std::marker::PhantomData<*const ()>,
}

impl DeviceHandle {
    /// Create a new device handle from a reference
    pub fn new<T>(device: &T) -> Self {
        Self {
            ptr: device as *const T as *const (),
            _marker: std::marker::PhantomData,
        }
    }

    /// Get the raw pointer
    pub fn as_ptr<T>(&self) -> *const T {
        self.ptr as *const T
    }

    /// Get the mutable raw pointer
    pub fn as_mut_ptr<T>(&self) -> *mut T {
        self.ptr as *mut T
    }
}

unsafe impl Send for DeviceHandle {}
unsafe impl Sync for DeviceHandle {}

/// Opaque handle to a GPU queue
#[derive(Clone, Copy)]
pub struct QueueHandle {
    pub(crate) ptr: *const (),
    pub(crate) _marker: std::marker::PhantomData<*const ()>,
}

impl QueueHandle {
    /// Create a new queue handle from a reference
    pub fn new<T>(queue: &T) -> Self {
        Self {
            ptr: queue as *const T as *const (),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn as_ptr<T>(&self) -> *const T {
        self.ptr as *const T
    }

    pub fn as_mut_ptr<T>(&self) -> *mut T {
        self.ptr as *mut T
    }
}

unsafe impl Send for QueueHandle {}
unsafe impl Sync for QueueHandle {}

/// Opaque handle to a surface target (window/native surface)
pub struct SurfaceTargetHandle {
    pub(crate) inner: Box<dyn Any + Send + Sync>,
}

impl SurfaceTargetHandle {
    pub fn new<T: Any + Send + Sync>(target: T) -> Self {
        Self {
            inner: Box::new(target),
        }
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }

    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.inner.downcast_mut::<T>()
    }

    pub fn into_inner<T: Any>(self) -> Option<T> {
        self.inner.downcast::<T>().ok().map(|b| *b)
    }
}

/// Opaque handle to a surface
pub struct SurfaceHandle {
    pub(crate) inner: Box<dyn Any + Send + Sync>,
}

impl SurfaceHandle {
    pub fn new<T: Any + Send + Sync>(surface: T) -> Self {
        Self {
            inner: Box::new(surface),
        }
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }

    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.inner.downcast_mut::<T>()
    }

    /// Consume the handle and return the inner value
    pub fn into_inner<T: Any>(self) -> Option<T> {
        self.inner.downcast::<T>().ok().map(|b| *b)
    }
}

/// Render context for managing devices and surfaces
///
/// This is an opaque type that wraps the backend-specific render context.
/// For Vello backend, this wraps `vello::util::RenderContext`.
pub struct RenderContext {
    pub(crate) inner: Box<dyn Any + Send + Sync>,
}

impl RenderContext {
    pub fn new<T: Any + Send + Sync>(ctx: T) -> Self {
        Self {
            inner: Box::new(ctx),
        }
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }

    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.inner.downcast_mut::<T>()
    }

    pub fn into_inner<T: Any>(self) -> Option<T> {
        self.inner.downcast::<T>().ok().map(|b| *b)
    }
}

/// Surface state trait - implemented by backend-specific surface states
#[cfg(not(target_arch = "wasm32"))]
pub trait SurfaceState: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn resize(&mut self, context: &mut RenderContext, width: u32, height: u32);
    fn width(&self) -> u32;
    fn height(&self) -> u32;
}

#[cfg(target_arch = "wasm32")]
pub trait SurfaceState {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn resize(&mut self, context: &mut RenderContext, width: u32, height: u32);
    fn width(&self) -> u32;
    fn height(&self) -> u32;
}

/// Lifecycle events for the renderer
#[derive(Debug, Clone, Copy)]
pub enum LifecycleEvent {
    FirstFrameDone,
    Suspend,
    Shutdown,
}

/// Backend configuration
pub struct BackendConfig {
    pub data_dir: String,
}

/// Render result type
pub type RenderResult = anyhow::Result<()>;

/// Classification for frame outcomes used by the CadenceGovernor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameResultClass {
    /// Frame presented successfully
    OnTime,
    /// Frame missed its cadence tick
    MissedCadence,
    /// No new content to present
    SkippedIdle,
    /// Skipped due to cadence divisor
    SkippedDivisor,
    /// Skipped because another frame was still rendering
    SkippedInFlight,
}

#[derive(Clone, Debug)]
pub struct ShadowDesc {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    /// RGBA, 8-bit per channel (sRGB, non-premultiplied).
    pub color: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct TextGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug)]
pub struct TextGlyphRun {
    /// Neutral font resource — backend resolves to native type via internal cache.
    pub font_data: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    pub font_size: f32,
    /// RGBA, 8-bit per channel (sRGB, non-premultiplied).
    pub color: [u8; 4],
    pub glyphs: Vec<TextGlyph>,
}

#[derive(Clone, Debug)]
pub struct TextDecoration {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// RGBA, 8-bit per channel (sRGB, non-premultiplied).
    pub color: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct PreparedText {
    pub glyph_runs: Vec<TextGlyphRun>,
    pub decorations: Vec<TextDecoration>,
}

#[derive(Clone, Debug)]
pub struct TextDrawPayload {
    pub node_id: u32,
    pub text: String,
    pub font_size: f32,
    pub font_family: String,
    pub font_weight: u16,
    /// RGBA, 8-bit per channel (sRGB, non-premultiplied).
    pub text_color: [u8; 4],
    pub measured_width: f32,
    pub measured_height: f32,
    pub prepared: PreparedText,
}

#[derive(Clone, Debug)]
pub enum NodeContent {
    /// RGBA, 8-bit per channel (sRGB, non-premultiplied).
    Rect { color: [u8; 4] },
    Text(TextDrawPayload),
}

#[derive(Clone, Debug)]
pub struct BlurEffect {
    pub node_id: u32,
    pub local_transform: Transform,
    pub width: f64,
    pub height: f64,
    pub blur_radius: f32,
    pub blur_style: u8,
    pub opacity: f32,
    /// RGBA, 8-bit per channel (sRGB, non-premultiplied).
    pub overlay_color: [u8; 4],
    pub border_radius: f32,
    pub source_rect: (f32, f32, f32, f32),
    pub deferred_children: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct SceneNode {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub position_x: f32,
    pub position_y: f32,
    pub content: NodeContent,
    pub border_radius: f32,
    pub opacity: f32,
    pub clip_to_bounds: bool,
    pub shadow: Option<ShadowDesc>,
    pub blur: Option<BlurEffect>,
    pub children: Vec<u32>,
}

/// Bake plan — an explicit instruction from Runtime to Backend to render a
/// specific node subtree into a GPU texture for raster caching.
#[derive(Clone, Debug)]
pub struct BakePlan {
    pub node_id: u32,
    pub width: u32,
    pub height: u32,
}

/// Recycle plan — an explicit instruction from Runtime to Backend to release
/// a cached GPU texture for a given node.
#[derive(Clone, Debug)]
pub struct RecyclePlan {
    pub node_id: u32,
    pub texture_id: TextureId,
}

/// Render package - all data needed for a single frame render.
///
/// This is produced by the Runtime layer and consumed by the RenderBackend.
/// It separates "what to render" (prepared by Runtime) from "how to render it"
/// (executed by the backend).
#[derive(Clone)]
pub struct RenderPackage {
    /// Viewport dimensions in pixels
    pub viewport: (u32, u32),
    /// Root node id (if any)
    pub root_id: Option<u32>,
    /// Flattened scene snapshot produced by Runtime
    pub nodes: Vec<SceneNode>,
    /// Epoch incremented whenever layout is recomputed
    pub layout_epoch: u64,
    /// Set to true when the Runtime performed layout this frame
    pub did_layout: bool,
    /// Dirty tracker snapshot for this frame (used by Runtime cache policy)
    pub dirty_tracker: crate::dirty::DirtyTracker,
    /// Explicit bake plans produced by Runtime cache policy.
    /// Backend executes these by rendering each node subtree into a GPU texture.
    pub bake_plans: Vec<BakePlan>,
    /// Texture IDs to recycle this frame (produced by Runtime cache policy).
    /// Backend releases these from its GPU texture pool.
    pub recycle_plans: Vec<RecyclePlan>,
}

impl RenderPackage {
    pub fn new(
        viewport: (u32, u32),
        root_id: Option<u32>,
        nodes: Vec<SceneNode>,
    ) -> Self {
        Self {
            viewport,
            root_id,
            nodes,
            layout_epoch: 0,
            did_layout: false,
            dirty_tracker: crate::dirty::DirtyTracker::new(),
            bake_plans: Vec::new(),
            recycle_plans: Vec::new(),
        }
    }
}

/// Render backend trait - implemented by concrete backends (Vello, Impeller, etc.)
#[cfg(not(target_arch = "wasm32"))]
pub trait RenderBackend: Send + Sync {
    /// Initialize the backend with GPU device and queue
    fn init(&self, device: DeviceHandle, queue: QueueHandle, config: BackendConfig)
        -> RenderResult;

    /// Create a surface state for rendering
    fn create_surface_state(
        &self,
        context: &mut RenderContext,
        target: Option<SurfaceTargetHandle>,
        surface: Option<SurfaceHandle>,
        surface_ptr: u64,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Box<dyn SurfaceState>>;

    /// Set frame timing data from the pacer (optional; default no-op)
    fn set_frame_timing(&self, _pacer_wait_ms: f64, _frame_interval_ms: f64) {}

    /// Set frame performance stats from the scheduler (optional; default no-op)
    fn set_frame_performance_stats(&self, _stats: dyxel_perf::FramePerformanceStats) {}

    /// Render a frame from a prepared package
    fn render_package(
        &self,
        _device: DeviceHandle,
        _queue: QueueHandle,
        _surface: &mut dyn SurfaceState,
        _package: &RenderPackage,
    ) -> RenderResult {
        Err(anyhow::anyhow!("render_package not implemented by backend"))
    }

    /// Handle lifecycle events
    fn on_lifecycle_event(&self, event: LifecycleEvent);

    /// Synchronize GPU (block until all work is done)
    fn sync_gpu(&self, device: DeviceHandle, queue: QueueHandle);

    /// Get as Any for downcasting
    fn as_any(&self) -> &dyn Any;
}

#[cfg(target_arch = "wasm32")]
pub trait RenderBackend {
    fn init(&self, device: DeviceHandle, queue: QueueHandle, config: BackendConfig)
        -> RenderResult;

    fn create_surface_state(
        &self,
        context: &mut RenderContext,
        target: Option<SurfaceTargetHandle>,
        surface: Option<SurfaceHandle>,
        surface_ptr: u64,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Box<dyn SurfaceState>>;

    /// Render a frame from a prepared package
    fn render_package(
        &self,
        _device: DeviceHandle,
        _queue: QueueHandle,
        _surface: &mut dyn SurfaceState,
        _package: &RenderPackage,
    ) -> RenderResult {
        Err(anyhow::anyhow!("render_package not implemented by backend"))
    }

    /// Set frame timing data from the pacer (optional; default no-op)
    fn set_frame_timing(&self, _pacer_wait_ms: f64, _frame_interval_ms: f64) {}

    /// Set frame performance stats from the scheduler (optional; default no-op)
    fn set_frame_performance_stats(&self, _stats: dyxel_perf::FramePerformanceStats) {}

    fn on_lifecycle_event(&self, event: LifecycleEvent);

    fn sync_gpu(&self, device: DeviceHandle, queue: QueueHandle);

    fn as_any(&self) -> &dyn Any;
}

/// Factory for creating render backends
pub trait RenderBackendFactory: Send + Sync {
    /// Create a new render backend instance
    fn create(&self) -> Box<dyn RenderBackend>;

    /// Get the name of this backend
    fn name(&self) -> &'static str;
}

/// Scene interface for drawing - abstracted from vello::Scene
///
/// This provides a minimal interface for drawing primitives.
/// Concrete backends implement this for their scene types.
pub trait Scene {
    /// Fill a rectangle with a color
    fn fill_rect(&mut self, x: f64, y: f64, width: f64, height: f64, color: [u8; 4]);

    /// Fill a rounded rectangle
    fn fill_rounded_rect(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        radius: f64,
        color: [u8; 4],
    );

    /// Apply a transform to subsequent drawing operations
    fn push_transform(&mut self, transform: Transform);

    /// Pop the last transform
    fn pop_transform(&mut self);

    /// Clear the scene
    fn clear(&mut self);

    /// Push a new layer for compositing effects (alpha, blend, filter, clip)
    ///
    /// All subsequent drawing operations will be captured in this layer
    /// until `pop_layer` is called. The layer will be composited with the
    /// specified effects applied.
    ///
    /// # Arguments
    /// * `alpha` - Layer opacity (0.0 to 1.0)
    /// * `blend` - Blend mode for compositing with parent
    /// * `filter` - Optional filter effect to apply
    /// * `clip` - Optional clip rectangle (in local coordinates)
    fn push_layer(
        &mut self,
        alpha: f32,
        blend: BlendMode,
        filter: Option<&Filter>,
        clip: Option<Rect>,
    );

    /// Pop the current layer and composite it
    ///
    /// This ends the layer scope started by `push_layer` and composites
    /// the captured content with the effects specified in push_layer.
    fn pop_layer(&mut self);
}

/// 2D Transform (affine transformation)
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub xx: f64,
    pub yx: f64,
    pub xy: f64,
    pub yy: f64,
    pub x0: f64,
    pub y0: f64,
}

impl Transform {
    /// Identity transform
    pub const IDENTITY: Self = Self {
        xx: 1.0,
        yx: 0.0,
        xy: 0.0,
        yy: 1.0,
        x0: 0.0,
        y0: 0.0,
    };

    /// Create a translation transform
    pub fn translate(x: f64, y: f64) -> Self {
        Self {
            xx: 1.0,
            yx: 0.0,
            xy: 0.0,
            yy: 1.0,
            x0: x,
            y0: y,
        }
    }

    /// Create a scale transform
    pub fn scale(sx: f64, sy: f64) -> Self {
        Self {
            xx: sx,
            yx: 0.0,
            xy: 0.0,
            yy: sy,
            x0: 0.0,
            y0: 0.0,
        }
    }

    /// Create a non-uniform scale
    pub fn scale_non_uniform(sx: f64, sy: f64) -> Self {
        Self::scale(sx, sy)
    }

    /// Multiply two transforms: self * other
    pub fn then(&self, other: &Self) -> Self {
        Self {
            xx: self.xx * other.xx + self.yx * other.xy,
            yx: self.xx * other.yx + self.yx * other.yy,
            xy: self.xy * other.xx + self.yy * other.xy,
            yy: self.xy * other.yx + self.yy * other.yy,
            x0: self.x0 * other.xx + self.y0 * other.xy + other.x0,
            y0: self.x0 * other.yx + self.y0 * other.yy + other.y0,
        }
    }
}

/// Render backend extensions for specific capabilities
pub trait RenderBackendExt {
    /// Enable performance overlay (if supported)
    fn enable_perf_overlay(&self);

    /// Disable performance overlay
    fn disable_perf_overlay(&self);
}

/// Utility trait for downcasting backend implementations
pub trait AsRenderBackend {
    fn as_vello_backend(&self) -> Option<&dyn VelloBackendExt>;
}

/// Extension trait for Vello-specific functionality
///
/// This is only available when the Vello backend is used.
/// Callers should check if backend implements this trait for Vello-specific features.
pub trait VelloBackendExt: RenderBackend {
    /// Get access to internal Vello renderer (for advanced use cases)
    fn vello_renderer(&self) -> Option<&dyn Any>;
}

// =============================================================================
// Double-Layer Backend Architecture (GraphicsRuntime + RenderBackend)
// =============================================================================
// This is the new backend abstraction introduced as part of the
// "Render Backend Extreme Decoupling" redesign. The old single-layer
// RenderBackend trait above is kept as a transition layer.
//
// New flow:
//   Platform -> NativeSurfaceHandle -> GraphicsRuntime -> BackendFrameContext
//                                                         -> RenderBackend
// =============================================================================

/// Capability flags for a backend — pure data, no downcast needed.
#[derive(Clone, Debug)]
pub struct BackendCapabilities {
    pub perf_overlay: bool,
    pub gpu_timing: bool,
    pub renderer_warmup: bool,
    pub main_thread_surface_creation: bool,
    pub main_thread_rendering: bool,
    pub explicit_present: bool,
}

/// Runtime family for compatibility check between runtime and backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeKind {
    Wgpu,
    Impeller,
    Skia,
}

/// Platform-native surface handle — NOT backend-specific.
///
/// Platform layers construct this and pass it to GraphicsRuntime.
/// The runtime is responsible for converting it to a concrete surface object.
pub enum NativeSurfaceHandle {
    /// Standard raw-window-handle path for desktop platforms
    RawWindow {
        window: raw_window_handle::RawWindowHandle,
        display: raw_window_handle::RawDisplayHandle,
    },
    /// Web canvas path
    WebCanvas {
        canvas_id: String,
    },
    /// Opaque native surface pointer for platforms that cannot use
    /// raw-window-handle naturally (e.g. Android ANativeWindow*)
    NativeSurface {
        kind: NativeSurfaceKind,
        ptr: u64,
    },
}

// Native surface handles are opaque transport values passed between platform
// threads and the render runtime. They do not provide safe dereference on their
// own; concrete runtimes remain responsible for honoring platform thread rules.
unsafe impl Send for NativeSurfaceHandle {}
unsafe impl Sync for NativeSurfaceHandle {}

/// Surface kind for NativeSurfaceHandle::NativeSurface
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeSurfaceKind {
    Android,
    Ios,
    Other,
}

/// Opaque surface identifier returned by GraphicsRuntime::create_surface()
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeSurfaceId(pub u32);

/// Per-frame rendering context — created by GraphicsRuntime, consumed by RenderBackend.
pub trait BackendFrameContext {
    fn as_any(&mut self) -> &mut dyn Any;
    fn runtime_kind(&self) -> RuntimeKind;
}

/// Graphics runtime — manages platform graphics runtime lifecycle.
///
/// Responsibilities:
/// - Create/resize/suspend/resume surfaces
/// - Acquire/release per-frame contexts (begin_frame / end_frame)
/// - GPU synchronization
/// - Present / swapbuffers
///
/// Does NOT execute scene drawing. Drawing is RenderBackend's job.
#[cfg(not(target_arch = "wasm32"))]
pub trait GraphicsRuntime: Send + Sync {
    fn initialize(&mut self) -> anyhow::Result<()>;

    fn create_surface(
        &mut self,
        handle: NativeSurfaceHandle,
        width: u32,
        height: u32,
    ) -> anyhow::Result<RuntimeSurfaceId>;

    fn resize_surface(
        &mut self,
        surface: RuntimeSurfaceId,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()>;

    fn suspend(&mut self) -> anyhow::Result<()>;
    fn resume(&mut self) -> anyhow::Result<()>;
    fn sync_gpu(&mut self) -> anyhow::Result<()>;

    fn begin_frame(
        &mut self,
        surface: RuntimeSurfaceId,
    ) -> anyhow::Result<Box<dyn BackendFrameContext>>;

    fn end_frame(
        &mut self,
        frame: Box<dyn BackendFrameContext>,
    ) -> anyhow::Result<()>;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[cfg(target_arch = "wasm32")]
pub trait GraphicsRuntime {
    fn initialize(&mut self) -> anyhow::Result<()>;

    fn create_surface(
        &mut self,
        handle: NativeSurfaceHandle,
        width: u32,
        height: u32,
    ) -> anyhow::Result<RuntimeSurfaceId>;

    fn resize_surface(
        &mut self,
        surface: RuntimeSurfaceId,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()>;

    fn suspend(&mut self) -> anyhow::Result<()>;
    fn resume(&mut self) -> anyhow::Result<()>;
    fn sync_gpu(&mut self) -> anyhow::Result<()>;

    fn begin_frame(
        &mut self,
        surface: RuntimeSurfaceId,
    ) -> anyhow::Result<Box<dyn BackendFrameContext>>;

    fn end_frame(
        &mut self,
        frame: Box<dyn BackendFrameContext>,
    ) -> anyhow::Result<()>;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Render backend — consumes RenderPackage and executes drawing.
///
/// Responsibilities:
/// - Scene draw / text / blur / shadow / layer
/// - Raster cache bake / recycle execution
/// - Backend-specific overlay / timing
///
/// Does NOT manage surfaces, devices, or present. Those are GraphicsRuntime's job.
#[cfg(not(target_arch = "wasm32"))]
pub trait RenderBackendV2: Send + Sync {
    fn initialize(&mut self, runtime: &mut dyn GraphicsRuntime) -> anyhow::Result<()>;

    fn render(
        &mut self,
        frame: &mut dyn BackendFrameContext,
        package: &RenderPackage,
    ) -> anyhow::Result<RenderFrameStats>;

    fn set_frame_timing(&self, _pacer_wait_ms: f64, _frame_interval_ms: f64) {}

    fn set_frame_performance_stats(&self, _stats: dyxel_perf::FramePerformanceStats) {}

    fn on_lifecycle_event(&self, event: LifecycleEvent) -> anyhow::Result<()> {
        let _ = event;
        Ok(())
    }

    fn enable_perf_overlay(&self) {}
}

#[cfg(target_arch = "wasm32")]
pub trait RenderBackendV2 {
    fn initialize(&mut self, runtime: &mut dyn GraphicsRuntime) -> anyhow::Result<()>;

    fn render(
        &mut self,
        frame: &mut dyn BackendFrameContext,
        package: &RenderPackage,
    ) -> anyhow::Result<RenderFrameStats>;

    fn set_frame_timing(&self, _pacer_wait_ms: f64, _frame_interval_ms: f64) {}

    fn set_frame_performance_stats(&self, _stats: dyxel_perf::FramePerformanceStats) {}

    fn on_lifecycle_event(&self, event: LifecycleEvent) -> anyhow::Result<()> {
        let _ = event;
        Ok(())
    }

    fn enable_perf_overlay(&self) {}
}

/// Factory for creating runtime + backend pairs at compile time.
pub trait GraphicsRuntimeFactory: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn capabilities(&self) -> BackendCapabilities;
    fn create_runtime(&self) -> anyhow::Result<Box<dyn GraphicsRuntime>>;
    fn create_backend(&self) -> anyhow::Result<Box<dyn RenderBackendV2>>;
}

/// Neutral frame timing stats returned by backend to scheduler/perf.
pub struct RenderFrameStats {
    pub cpu_time_ms: Option<f64>,
    pub gpu_time_ms: Option<f64>,
    pub backend_internal_stats: Option<serde_json::Value>,
}
