// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_render_api::{RenderContext, SurfaceState};
use impellers::Context;
use objc::{msg_send, runtime::Object, sel, sel_impl};
use std::os::raw::c_void;

pub struct MacImpellerSurfaceState {
    pub layer: *mut Object,
    pub width: u32,
    pub height: u32,
}

impl MacImpellerSurfaceState {
    pub fn new(ns_view: *mut c_void, width: u32, height: u32) -> Self {
        unsafe {
            use metal::foreign_types::ForeignType;
            let device = metal::Device::system_default().expect("No Metal device found");
            let device_ptr: *mut Object = device.as_ptr() as _;

            let layer: *mut Object = msg_send![objc::class!(CAMetalLayer), new];

            let _: () = msg_send![layer, setDevice: device_ptr];
            let _: () = msg_send![layer, setPixelFormat: 80]; // MTLPixelFormatBGRA8Unorm
            let _: () = msg_send![layer, setPresentsWithTransaction: false];

            let view: *mut Object = ns_view as _;

            // Get scale factor
            let window: *mut Object = msg_send![view, window];
            let scale_factor: f64 = if !window.is_null() {
                msg_send![window, backingScaleFactor]
            } else {
                1.0
            };
            let _: () = msg_send![layer, setContentsScale: scale_factor];

            let _: () = msg_send![view, setWantsLayer: true];
            let _: () = msg_send![view, setLayer: layer];

            Self {
                layer,
                width,
                height,
            }
        }
    }
}

unsafe impl Send for MacImpellerSurfaceState {}
unsafe impl Sync for MacImpellerSurfaceState {}

impl SurfaceState for MacImpellerSurfaceState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn resize(&mut self, _context: &mut RenderContext, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        unsafe {
            let size = cocoa::foundation::NSSize::new(width as f64, height as f64);
            let _: () = msg_send![self.layer, setDrawableSize: size];
        }
    }
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
}

pub fn render_mac(
    context: &mut Context,
    surface: &mut MacImpellerSurfaceState,
    display_list: &impellers::DisplayList,
) -> anyhow::Result<()> {
    unsafe {
        let drawable: *mut Object = msg_send![surface.layer, nextDrawable];
        if !drawable.is_null() {
            let drawable_ptr = drawable as *mut c_void;
            if let Some(mut impeller_surface) = context.wrap_metal_drawable(drawable_ptr) {
                let _ = impeller_surface.draw_display_list(display_list);
                let _ = impeller_surface.present();
            }
        }
    }
    Ok(())
}
