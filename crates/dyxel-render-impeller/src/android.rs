// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use ash::vk::Handle;
use dyxel_render_api::{RenderContext, SurfaceState};
use impellers::{Context, VkSwapChain};
use std::os::raw::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

static ANDROID_IMPELLER_RENDER_COUNT: AtomicU32 = AtomicU32::new(0);
static ANDROID_IMPELLER_ACQUIRE_FAIL_COUNT: AtomicU32 = AtomicU32::new(0);

pub struct AndroidImpellerSurfaceState {
    pub swapchain: VkSwapChain,
    pub width: u32,
    pub height: u32,
    pub density: f32,
}

impl AndroidImpellerSurfaceState {
    pub fn new(
        context: &Context,
        native_window: *mut c_void,
        width: u32,
        height: u32,
        density: f32,
    ) -> Self {
        unsafe {
            // 1. Get Vulkan instance from Impeller Context
            let vk_info = context.get_vulkan_info().expect("Not a Vulkan context");
            // 2. Load Ash Entry & Instance
            let entry = ash::Entry::load().expect("Failed to load Vulkan entry");
            // Workaround to create ash::Instance from a raw handle
            let vk_instance_handle = ash::vk::Instance::from_raw(vk_info.vk_instance as u64);
            let instance = ash::Instance::load(entry.static_fn(), vk_instance_handle);

            // 3. Create Android Surface
            let android_surface_fn = ash::khr::android_surface::Instance::new(&entry, &instance);
            let create_info =
                ash::vk::AndroidSurfaceCreateInfoKHR::default().window(native_window as *mut _);

            let vk_surface = android_surface_fn
                .create_android_surface(&create_info, None)
                .expect("Failed to create Android Vulkan Surface");

            // 4. Create Impeller Swapchain from the VkSurfaceKHR
            let swapchain = context
                .create_new_vulkan_swapchain(vk_surface.as_raw() as *mut c_void)
                .expect("Failed to create Impeller VkSwapChain");

            Self {
                swapchain,
                width,
                height,
                density,
            }
        }
    }
}

unsafe impl Send for AndroidImpellerSurfaceState {}
unsafe impl Sync for AndroidImpellerSurfaceState {}

impl SurfaceState for AndroidImpellerSurfaceState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn resize(&mut self, _context: &mut RenderContext, width: u32, height: u32) {
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

pub fn render_android(
    _context: &mut Context,
    surface: &mut AndroidImpellerSurfaceState,
    display_list: &impellers::DisplayList,
) -> anyhow::Result<()> {
    let entry_count = ANDROID_IMPELLER_RENDER_COUNT.load(Ordering::Relaxed);
    if entry_count < 5 {
        log::info!(
            "RENDER_ANDROID_ENTRY: width={}, height={}",
            surface.width,
            surface.height
        );
    }

    if let Some(mut impeller_surface) = surface.swapchain.acquire_next_surface_new() {
        let render_count = ANDROID_IMPELLER_RENDER_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if render_count <= 5 {
            log::info!("RENDER_ANDROID: Surface acquired, drawing...");
        }
        if let Err(e) = impeller_surface.draw_display_list(display_list) {
            log::error!("IMPELLER: draw_display_list failed: {:?}", e);
        }
        if let Err(e) = impeller_surface.present() {
            log::error!("IMPELLER: present failed: {:?}", e);
        }
    } else {
        let fail_count = ANDROID_IMPELLER_ACQUIRE_FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
        if fail_count % 60 == 0 {
            log::warn!(
                "IMPELLER: Failed to acquire next surface from swapchain (count: {})",
                fail_count
            );
        }
    }
    Ok(())
}
