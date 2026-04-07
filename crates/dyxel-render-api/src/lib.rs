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
use dyxel_shared::{SharedState, filters::{BlendMode, Filter, Rect}};

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

/// Render backend trait - implemented by concrete backends (Vello, Impeller, etc.)
#[cfg(not(target_arch = "wasm32"))]
pub trait RenderBackend: Send + Sync {
    /// Initialize the backend with GPU device and queue
    fn init(&self, device: DeviceHandle, queue: QueueHandle, config: BackendConfig) -> RenderResult;
    
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
    
    /// Prepare for rendering (called before render)
    fn prepare(&self, shared_state: &SharedPtr<SharedMutex<SharedState>>, width: u32, height: u32);
    
    /// Render a frame
    fn render(
        &self,
        device: DeviceHandle,
        queue: QueueHandle,
        surface: &mut dyn SurfaceState,
        shared_state: &SharedPtr<SharedMutex<SharedState>>,
    ) -> RenderResult;
    
    /// Handle lifecycle events
    fn on_lifecycle_event(&self, event: LifecycleEvent);
    
    /// Synchronize GPU (block until all work is done)
    fn sync_gpu(&self, device: DeviceHandle, queue: QueueHandle);
    
    /// Get as Any for downcasting
    fn as_any(&self) -> &dyn Any;
}

#[cfg(target_arch = "wasm32")]
pub trait RenderBackend {
    fn init(&self, device: DeviceHandle, queue: QueueHandle, config: BackendConfig) -> RenderResult;
    
    fn create_surface_state(
        &self,
        context: &mut RenderContext,
        target: Option<SurfaceTargetHandle>,
        surface: Option<SurfaceHandle>,
        surface_ptr: u64,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Box<dyn SurfaceState>>;
    
    fn prepare(&self, shared_state: &SharedPtr<SharedMutex<SharedState>>, width: u32, height: u32);
    
    fn render(
        &self,
        device: DeviceHandle,
        queue: QueueHandle,
        surface: &mut dyn SurfaceState,
        shared_state: &SharedPtr<SharedMutex<SharedState>>,
    ) -> RenderResult;
    
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
    fn fill_rounded_rect(&mut self, x: f64, y: f64, width: f64, height: f64, radius: f64, color: [u8; 4]);

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
    pub xx: f64, pub yx: f64,
    pub xy: f64, pub yy: f64,
    pub x0: f64, pub y0: f64,
}

impl Transform {
    /// Identity transform
    pub const IDENTITY: Self = Self {
        xx: 1.0, yx: 0.0,
        xy: 0.0, yy: 1.0,
        x0: 0.0, y0: 0.0,
    };
    
    /// Create a translation transform
    pub fn translate(x: f64, y: f64) -> Self {
        Self {
            xx: 1.0, yx: 0.0,
            xy: 0.0, yy: 1.0,
            x0: x, y0: y,
        }
    }
    
    /// Create a scale transform
    pub fn scale(sx: f64, sy: f64) -> Self {
        Self {
            xx: sx, yx: 0.0,
            xy: 0.0, yy: sy,
            x0: 0.0, y0: 0.0,
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

/// Text rendering interface
/// 
/// Abstraction for text layout and rendering
pub trait TextRenderer {
    /// Set text content
    fn set_text(&mut self, text: &str);
    
    /// Set font size
    fn set_font_size(&mut self, size: f32);
    
    /// Set text color
    fn set_text_color(&mut self, r: u8, g: u8, b: u8, a: u8);
    
    /// Set layout width (for wrapping)
    fn set_width(&mut self, width: Option<f32>);
    
    /// Get layout size
    fn layout_size(&mut self) -> (f32, f32);
    
    /// Draw text to a scene
    fn draw(&mut self, scene: &mut dyn Scene, transform: Transform);
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
