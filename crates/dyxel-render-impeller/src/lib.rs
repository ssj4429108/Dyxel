// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Allow unexpected_cfgs from objc crate macros
#![allow(unexpected_cfgs)]

use dyxel_render_api::{
    BackendConfig, DeviceHandle, QueueHandle, RenderPackage, RenderResult, SurfaceHandle,
    SurfaceTargetHandle,
};
use dyxel_render_api::{LifecycleEvent, RenderBackend, RenderContext, SurfaceState};
use dyxel_shared::SharedState;
use impellers::{Color, Context, DisplayListBuilder, Paint, Point, Rect, Size};
use kurbo::Vec2;
use std::sync::Mutex;

#[cfg(target_os = "macos")]
pub mod mac;

#[cfg(target_os = "android")]
pub mod android;

pub struct ImpellerBackend {
    context: Mutex<Option<Context>>,
}

unsafe impl Send for ImpellerBackend {}
unsafe impl Sync for ImpellerBackend {}

impl ImpellerBackend {
    pub fn new() -> Self {
        Self {
            context: Mutex::new(None),
        }
    }
}

#[allow(dead_code)]
fn render_node_recursive(
    id: u32,
    state: &SharedState,
    builder: &mut DisplayListBuilder,
    parent_pos: Vec2,
) {
    if let Some(node) = state.nodes.get(&id) {
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let global_pos = parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);

        let mut paint = Paint::default();
        let c = node.color.to_rgba8();
        paint.set_color(Color::new_srgba(
            c.r as f32 / 255.0,
            c.g as f32 / 255.0,
            c.b as f32 / 255.0,
            1.0,
        ));

        // Probe experiment logic:
        if id == 0 {
            // 1. Force shrink root node to 100x100, observe its physical size on screen
            let rect = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
            builder.draw_rect(&rect, &paint);
            log::info!(
                "IMPELLER: Drawing Root (ID 0) forced to 100x100 at (0,0). Children: {}",
                node.children.len()
            );
        } else {
            // 2. Child nodes: draw at (150 + offset, 150 + offset)
            let rect = Rect::new(
                Point::new(
                    150.0 + global_pos.x as f32 * 0.1,
                    150.0 + global_pos.y as f32 * 0.1,
                ),
                Size::new(50.0, 50.0),
            );
            // Bright yellow
            let mut p = Paint::default();
            p.set_color(Color::new_srgba(1.0, 1.0, 0.0, 1.0));
            builder.draw_rect(&rect, &p);
        }

        for &child_id in &node.children {
            render_node_recursive(child_id, state, builder, global_pos);
        }
    }
}

impl RenderBackend for ImpellerBackend {
    fn init(
        &self,
        _device: DeviceHandle,
        _queue: QueueHandle,
        _config: BackendConfig,
    ) -> anyhow::Result<()> {
        log::info!("IMPELLER_BACKEND: Initializing...");
        let context = unsafe {
            #[cfg(target_os = "macos")]
            {
                Context::new_metal()
            }
            #[cfg(target_os = "android")]
            {
                let lib = libloading::Library::new("libvulkan.so")
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?;
                Context::new_vulkan(false, move |_instance, name| {
                    let name_str = std::ffi::CStr::from_ptr(name).to_str().unwrap_or("");
                    match lib.get::<*mut std::os::raw::c_void>(name_str.as_bytes()) {
                        Ok(sym) => *sym,
                        Err(_) => std::ptr::null_mut(),
                    }
                })
            }
            #[cfg(all(
                not(target_os = "macos"),
                not(target_os = "android"),
                not(target_arch = "wasm32")
            ))]
            {
                Context::new_opengl_es(|_| std::ptr::null_mut())
            }
            #[cfg(target_arch = "wasm32")]
            {
                return Err(anyhow::anyhow!("Impeller not supported on WASM"));
            }
        }
        .map_err(|e| anyhow::anyhow!("Failed to create Impeller context: {:?}", e))?;
        *self.context.lock().unwrap() = Some(context);
        log::info!("IMPELLER_BACKEND: Initialization complete!");
        Ok(())
    }

    fn create_surface_state(
        &self,
        _ctx: &mut RenderContext,
        _target: Option<SurfaceTargetHandle>,
        _surface: Option<SurfaceHandle>,
        _ptr: u64,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Box<dyn SurfaceState>> {
        #[cfg(target_os = "android")]
        {
            if _ptr != 0 {
                let ctx_lock = self.context.lock().unwrap();
                let impeller_ctx = ctx_lock
                    .as_ref()
                    .expect("Impeller context must be initialized");
                return Ok(Box::new(android::AndroidImpellerSurfaceState::new(
                    impeller_ctx,
                    _ptr as *mut _,
                    width,
                    height,
                    density,
                )));
            }
        }
        Ok(Box::new(ImpellerSurfaceState { width, height }))
    }

    fn render_package(
        &self,
        _device: DeviceHandle,
        _queue: QueueHandle,
        _surface: &mut dyn SurfaceState,
        _package: &RenderPackage,
    ) -> RenderResult {
        Err(anyhow::anyhow!("Impeller backend not yet implemented"))
    }

    fn sync_gpu(&self, _device: DeviceHandle, _queue: QueueHandle) {
        // Impeller handles its own synchronization
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn on_lifecycle_event(&self, _event: LifecycleEvent) {}
}

pub struct ImpellerSurfaceState {
    pub width: u32,
    pub height: u32,
}
unsafe impl Send for ImpellerSurfaceState {}
unsafe impl Sync for ImpellerSurfaceState {}
impl SurfaceState for ImpellerSurfaceState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn resize(&mut self, _ctx: &mut RenderContext, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
}
