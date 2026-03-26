// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::any::Any;
pub use vello::wgpu;
pub use vello::util::RenderContext;
use dyxel_shared::SharedState;

// Platform-specific types
#[cfg(not(target_arch = "wasm32"))]
pub type SharedPtr<T> = std::sync::Arc<T>;
#[cfg(target_arch = "wasm32")]
pub type SharedPtr<T> = std::rc::Rc<T>;

#[cfg(not(target_arch = "wasm32"))]
pub type SharedMutex<T> = std::sync::Mutex<T>;
#[cfg(target_arch = "wasm32")]
pub type SharedMutex<T> = std::cell::RefCell<T>;

// Helper trait for unified lock API - provides lock().unwrap() compatibility
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

// For non-WASM, std::sync::Mutex already has lock(), we just need to make unwrap() work
#[cfg(not(target_arch = "wasm32"))]
pub trait LockExt<T> {}
#[cfg(not(target_arch = "wasm32"))]
impl<T> LockExt<T> for std::sync::Mutex<T> {}

#[cfg(not(target_arch = "wasm32"))]
pub trait RenderBackend: Send + Sync {
    fn init(&self, device: &wgpu::Device, queue: &wgpu::Queue, config: BackendConfig) -> anyhow::Result<()>;
    fn create_surface_state(&self, context: &mut RenderContext, target: Option<wgpu::SurfaceTarget<'static>>, surface: Option<wgpu::Surface<'static>>, surface_ptr: u64, width: u32, height: u32) -> anyhow::Result<Box<dyn SurfaceState>>;
    fn prepare(&self, shared_state: &SharedPtr<SharedMutex<SharedState>>, width: u32, height: u32);
    fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, surface: &mut dyn SurfaceState, shared_state: &SharedPtr<SharedMutex<SharedState>>) -> RenderResult;
    fn on_lifecycle_event(&self, event: LifecycleEvent);
    fn sync_gpu(&self, device: &wgpu::Device, queue: &wgpu::Queue);
}

#[cfg(target_arch = "wasm32")]
pub trait RenderBackend {
    fn init(&self, device: &wgpu::Device, queue: &wgpu::Queue, config: BackendConfig) -> anyhow::Result<()>;
    fn create_surface_state(&self, context: &mut RenderContext, target: Option<wgpu::SurfaceTarget<'static>>, surface: Option<wgpu::Surface<'static>>, surface_ptr: u64, width: u32, height: u32) -> anyhow::Result<Box<dyn SurfaceState>>;
    fn prepare(&self, shared_state: &SharedPtr<SharedMutex<SharedState>>, width: u32, height: u32);
    fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, surface: &mut dyn SurfaceState, shared_state: &SharedPtr<SharedMutex<SharedState>>) -> RenderResult;
    fn on_lifecycle_event(&self, event: LifecycleEvent);
    fn sync_gpu(&self, device: &wgpu::Device, queue: &wgpu::Queue);
}

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

#[derive(Debug, Clone, Copy)]
pub enum LifecycleEvent {
    FirstFrameDone,
    Suspend,
    Shutdown,
}

pub mod types {
    pub struct BackendConfig {
        pub data_dir: String,
    }
    pub type RenderResult = anyhow::Result<()>;
}

pub use types::*;
