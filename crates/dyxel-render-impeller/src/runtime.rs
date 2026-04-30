// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_render_api::{
    BackendFrameContext, GraphicsRuntime, NativeSurfaceHandle, NativeSurfaceKind, RuntimeKind,
    RuntimeSurfaceId,
};
use impellers::{Context, Surface, VkSwapChain};
use std::collections::HashMap;
use std::ffi::c_void;

#[cfg(target_os = "android")]
use ash::vk::Handle;
#[cfg(target_os = "android")]
use impellers::{ISize, PixelFormat};
#[cfg(target_os = "android")]
use khronos_egl as egl;

#[cfg(target_os = "android")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AndroidWsiMode {
    Gles,
    Vulkan,
}

#[cfg(target_os = "android")]
struct AndroidGlesSurface {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    surface: egl::Surface,
}

#[cfg(target_os = "android")]
impl AndroidGlesSurface {
    fn make_current(&self) -> anyhow::Result<()> {
        self.egl
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .map_err(|err| anyhow::anyhow!("eglMakeCurrent failed: {err:?}"))
    }

    fn swap_buffers(&self) -> anyhow::Result<()> {
        self.egl
            .swap_buffers(self.display, self.surface)
            .map_err(|err| anyhow::anyhow!("eglSwapBuffers failed: {err:?}"))
    }

    fn clear_current(&self) -> anyhow::Result<()> {
        self.egl
            .make_current(self.display, None, None, None)
            .map_err(|err| anyhow::anyhow!("eglMakeCurrent(NULL) failed: {err:?}"))
    }
}

#[cfg(target_os = "android")]
impl Drop for AndroidGlesSurface {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_surface(self.display, self.surface);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

struct ImpellerSurfaceRecord {
    swapchain: Option<VkSwapChain>,
    #[cfg(target_os = "android")]
    gles: Option<AndroidGlesSurface>,
    native_window: Option<*mut c_void>,
    width: u32,
    height: u32,
}

/// Impeller runtime: owns the Impeller context and platform swapchains.
///
/// Android support is intentionally minimal: Vulkan context + ANativeWindow
/// VkSurfaceKHR + Impeller VkSwapChain.
pub struct ImpellerRuntime {
    // Drop swapchains before the context.
    surfaces: HashMap<RuntimeSurfaceId, ImpellerSurfaceRecord>,
    context: Option<Context>,
    next_surface_id: u32,
    #[cfg(target_os = "android")]
    android_wsi_mode: AndroidWsiMode,
}

unsafe impl Send for ImpellerRuntime {}
unsafe impl Sync for ImpellerRuntime {}

impl ImpellerRuntime {
    pub fn new() -> Self {
        Self {
            surfaces: HashMap::new(),
            context: None,
            next_surface_id: 1,
            #[cfg(target_os = "android")]
            android_wsi_mode: android_wsi_mode(),
        }
    }
}

impl Drop for ImpellerRuntime {
    fn drop(&mut self) {
        #[cfg(target_os = "android")]
        if self.android_wsi_mode == AndroidWsiMode::Gles {
            // The Impeller GLES context must be released while the underlying
            // EGL context/window are still alive.
            self.context.take();
            self.surfaces.clear();
            return;
        }
        self.surfaces.clear();
        self.context.take();
    }
}

impl Default for ImpellerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-frame Impeller surface acquired from a swapchain.
pub struct ImpellerFrameContext {
    pub(crate) surface_id: RuntimeSurfaceId,
    pub(crate) surface: Option<Surface>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

unsafe impl Send for ImpellerFrameContext {}

impl BackendFrameContext for ImpellerFrameContext {
    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn runtime_kind(&self) -> RuntimeKind {
        RuntimeKind::Impeller
    }
}

impl GraphicsRuntime for ImpellerRuntime {
    fn initialize(&mut self) -> anyhow::Result<()> {
        #[cfg(target_os = "android")]
        if self.android_wsi_mode == AndroidWsiMode::Gles {
            log::info!("[DIAG-IMPELLER] runtime initialized with Android GLES WSI (lazy context)");
            return Ok(());
        }
        let context = create_platform_context()?;
        self.context = Some(context);
        log::info!("[DIAG-IMPELLER] runtime initialized");
        Ok(())
    }

    fn create_surface(
        &mut self,
        handle: NativeSurfaceHandle,
        width: u32,
        height: u32,
    ) -> anyhow::Result<RuntimeSurfaceId> {
        #[cfg(target_os = "android")]
        let (swapchain, gles, native_window) = match handle {
            NativeSurfaceHandle::NativeSurface {
                kind: NativeSurfaceKind::Android,
                ptr,
            } => {
                let native_window = ptr as *mut c_void;
                if self.android_wsi_mode == AndroidWsiMode::Gles {
                    let gles = create_android_gles_surface(native_window, width, height)?;
                    // Surface creation may happen on the Android/UI thread.
                    // Release the EGL context here so the render thread can
                    // bind it and create the Impeller GLES context there.
                    gles.clear_current()?;
                    (None, Some(gles), Some(native_window))
                } else {
                    let context = self
                        .context
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("ImpellerRuntime not initialized"))?;
                    (
                        Some(create_android_swapchain(
                            context,
                            native_window,
                            width,
                            height,
                        )?),
                        None,
                        Some(native_window),
                    )
                }
            }
            NativeSurfaceHandle::NativeSurface { kind, .. } => {
                return Err(anyhow::anyhow!(
                    "ImpellerRuntime unsupported native surface kind {:?}",
                    kind
                ));
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "ImpellerRuntime currently requires a native Android surface"
                ));
            }
        };
        #[cfg(not(target_os = "android"))]
        let (swapchain, native_window) = match handle {
            NativeSurfaceHandle::NativeSurface {
                kind: NativeSurfaceKind::Android,
                ptr,
            } => {
                let context = self
                    .context
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("ImpellerRuntime not initialized"))?;
                let native_window = ptr as *mut c_void;
                (
                    Some(create_android_swapchain(
                        context,
                        native_window,
                        width,
                        height,
                    )?),
                    Some(native_window),
                )
            }
            NativeSurfaceHandle::NativeSurface { kind, .. } => {
                return Err(anyhow::anyhow!(
                    "ImpellerRuntime unsupported native surface kind {:?}",
                    kind
                ));
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "ImpellerRuntime currently requires a native Android surface"
                ));
            }
        };

        let id = RuntimeSurfaceId(self.next_surface_id);
        self.next_surface_id += 1;
        self.surfaces.insert(
            id,
            ImpellerSurfaceRecord {
                swapchain,
                #[cfg(target_os = "android")]
                gles,
                native_window,
                width,
                height,
            },
        );
        log::info!(
            "[DIAG-IMPELLER] created surface {:?} size={}x{}",
            id,
            width,
            height
        );
        Ok(id)
    }

    fn resize_surface(
        &mut self,
        surface: RuntimeSurfaceId,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let record = self
            .surfaces
            .get_mut(&surface)
            .ok_or_else(|| anyhow::anyhow!("Impeller surface {:?} not found", surface))?;
        let size_changed = record.width != width || record.height != height;
        if size_changed {
            if let Some(native_window) = record.native_window {
                #[cfg(target_os = "android")]
                if record.gles.is_some() {
                    self.context.take();
                    record.gles.take();
                    let gles = create_android_gles_surface(native_window, width, height)?;
                    gles.clear_current()?;
                    record.gles = Some(gles);
                } else {
                    let context = self
                        .context
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("ImpellerRuntime not initialized"))?;
                    record.swapchain = Some(create_android_swapchain(
                        context,
                        native_window,
                        width,
                        height,
                    )?);
                }
                #[cfg(not(target_os = "android"))]
                {
                    let context = self
                        .context
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("ImpellerRuntime not initialized"))?;
                    record.swapchain = Some(create_android_swapchain(
                        context,
                        native_window,
                        width,
                        height,
                    )?);
                }
            }
        }
        record.width = width;
        record.height = height;
        log::info!(
            "[DIAG-IMPELLER] resized surface {:?} size={}x{} recreated={}",
            surface,
            width,
            height,
            size_changed
        );
        Ok(())
    }

    fn suspend(&mut self) -> anyhow::Result<()> {
        #[cfg(target_os = "android")]
        if self.android_wsi_mode == AndroidWsiMode::Gles {
            self.context.take();
        }
        self.surfaces.clear();
        Ok(())
    }

    fn resume(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn sync_gpu(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn begin_frame(
        &mut self,
        surface: RuntimeSurfaceId,
    ) -> anyhow::Result<Box<dyn BackendFrameContext>> {
        let record = self
            .surfaces
            .get_mut(&surface)
            .ok_or_else(|| anyhow::anyhow!("Impeller surface {:?} not found", surface))?;
        let acquire_t0 = std::time::Instant::now();
        #[cfg(target_os = "android")]
        let frame_surface = if let Some(gles) = record.gles.as_ref() {
            gles.make_current()?;
            if self.context.is_none() {
                self.context = Some(create_android_gles_context(gles)?);
            }
            let context = self
                .context
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("ImpellerRuntime not initialized"))?;
            unsafe {
                context.wrap_fbo(
                    0,
                    PixelFormat::RGBA8888,
                    ISize::new(record.width as i64, record.height as i64),
                )
            }
            .ok_or_else(|| anyhow::anyhow!("Impeller failed to wrap GLES default framebuffer"))?
        } else {
            record
                .swapchain
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Impeller swapchain missing"))?
                .acquire_next_surface_new()
                .ok_or_else(|| anyhow::anyhow!("Impeller failed to acquire swapchain surface"))?
        };
        #[cfg(not(target_os = "android"))]
        let frame_surface = record
            .swapchain
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Impeller swapchain missing"))?
            .acquire_next_surface_new()
            .ok_or_else(|| anyhow::anyhow!("Impeller failed to acquire swapchain surface"))?;
        let acquire_ms = acquire_t0.elapsed().as_secs_f64() * 1000.0;
        if acquire_ms >= 4.0 {
            log::info!("[DIAG-IMPELLER] acquire_ms={:.2}", acquire_ms);
        }
        Ok(Box::new(ImpellerFrameContext {
            surface_id: surface,
            surface: Some(frame_surface),
            width: record.width,
            height: record.height,
        }))
    }

    fn end_frame(&mut self, mut frame: Box<dyn BackendFrameContext>) -> anyhow::Result<()> {
        let frame = frame
            .as_any()
            .downcast_mut::<ImpellerFrameContext>()
            .ok_or_else(|| anyhow::anyhow!("Invalid Impeller frame context type"))?;
        let surface = frame
            .surface
            .take()
            .ok_or_else(|| anyhow::anyhow!("Impeller frame surface already consumed"))?;
        let present_t0 = std::time::Instant::now();
        #[cfg(target_os = "android")]
        let record = self
            .surfaces
            .get_mut(&frame.surface_id)
            .ok_or_else(|| anyhow::anyhow!("Impeller surface {:?} not found", frame.surface_id))?;
        #[cfg(target_os = "android")]
        if let Some(gles) = record.gles.as_ref() {
            gles.make_current()?;
            gles.swap_buffers()?;
            drop(surface);
        } else {
            surface
                .present()
                .map_err(|err| anyhow::anyhow!("Impeller present failed: {}", err))?;
        }
        #[cfg(not(target_os = "android"))]
        {
            surface
                .present()
                .map_err(|err| anyhow::anyhow!("Impeller present failed: {}", err))?;
        }
        let present_ms = present_t0.elapsed().as_secs_f64() * 1000.0;
        static PRESENT_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = PRESENT_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if count <= 5 || count % 60 == 0 || present_ms >= 8.0 {
            log::info!(
                "[DIAG-IMPELLER] presented frame count={} surface={:?} size={}x{} present_ms={:.2}",
                count,
                frame.surface_id,
                frame.width,
                frame.height,
                present_ms
            );
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(target_os = "android")]
fn android_wsi_mode() -> AndroidWsiMode {
    let value = android_system_property("debug.dyxel.impeller_wsi")
        .or_else(|| std::env::var("DYXEL_IMPELLER_ANDROID_WSI").ok())
        .unwrap_or_else(|| "gles".to_string());
    match value.trim().to_ascii_lowercase().as_str() {
        "vulkan" | "vk" => AndroidWsiMode::Vulkan,
        _ => AndroidWsiMode::Gles,
    }
}

#[cfg(target_os = "android")]
fn android_system_property(name: &str) -> Option<String> {
    use std::ffi::{CStr, CString};

    let key = CString::new(name).ok()?;
    let mut value = [0 as libc::c_char; libc::PROP_VALUE_MAX as usize];
    let len = unsafe { libc::__system_property_get(key.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return None;
    }
    let value = unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(target_os = "android")]
fn create_android_gles_context(gles: &AndroidGlesSurface) -> anyhow::Result<Context> {
    gles.make_current()?;
    let context = unsafe {
        Context::new_opengl_es(|name| {
            gles.egl
                .get_proc_address(name)
                .map(|proc| proc as *const () as *mut c_void)
                .unwrap_or(std::ptr::null_mut())
        })
    }
    .map_err(|err| anyhow::anyhow!("Failed to create Impeller OpenGLES context: {:?}", err))?;
    log::info!("[DIAG-IMPELLER] created Impeller OpenGLES context");
    Ok(context)
}

#[cfg(target_os = "android")]
fn create_android_gles_surface(
    native_window: *mut c_void,
    width: u32,
    height: u32,
) -> anyhow::Result<AndroidGlesSurface> {
    if native_window.is_null() {
        return Err(anyhow::anyhow!("Impeller GLES got null ANativeWindow"));
    }
    let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
        .map_err(|err| anyhow::anyhow!("Failed to load EGL: {err:?}"))?;
    let display = unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }
        .ok_or_else(|| anyhow::anyhow!("eglGetDisplay returned null"))?;
    let (major, minor) = egl
        .initialize(display)
        .map_err(|err| anyhow::anyhow!("eglInitialize failed: {err:?}"))?;
    egl.bind_api(egl::OPENGL_ES_API)
        .map_err(|err| anyhow::anyhow!("eglBindAPI(OpenGL ES) failed: {err:?}"))?;

    let config_attrs = [
        egl::SURFACE_TYPE,
        egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_ES2_BIT | egl::OPENGL_ES3_BIT,
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::STENCIL_SIZE,
        8,
        egl::DEPTH_SIZE,
        0,
        egl::NONE,
    ];
    let config = egl
        .choose_first_config(display, &config_attrs)
        .map_err(|err| anyhow::anyhow!("eglChooseConfig failed: {err:?}"))?
        .ok_or_else(|| anyhow::anyhow!("No EGL window config for Impeller GLES"))?;
    let visual_id = egl
        .get_config_attrib(display, config, egl::NATIVE_VISUAL_ID)
        .unwrap_or(0);
    configure_android_native_window_with_format(native_window, width, height, visual_id)?;

    let surface = unsafe {
        egl.create_window_surface(
            display,
            config,
            native_window as egl::NativeWindowType,
            None,
        )
    }
    .map_err(|err| anyhow::anyhow!("eglCreateWindowSurface failed: {err:?}"))?;

    let context = create_egl_context_es3_or_es2(&egl, display, config)?;
    let gles = AndroidGlesSurface {
        egl,
        display,
        context,
        surface,
    };
    gles.make_current()?;
    let surface_w = gles
        .egl
        .query_surface(display, surface, egl::WIDTH)
        .unwrap_or(0);
    let surface_h = gles
        .egl
        .query_surface(display, surface, egl::HEIGHT)
        .unwrap_or(0);
    log::info!(
        "[DIAG-IMPELLER] Android GLES WSI ready egl={}.{} req={}x{} surface={}x{} visual_id={}",
        major,
        minor,
        width,
        height,
        surface_w,
        surface_h,
        visual_id
    );
    Ok(gles)
}

#[cfg(target_os = "android")]
fn create_egl_context_es3_or_es2(
    egl: &egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    config: egl::Config,
) -> anyhow::Result<egl::Context> {
    let es3_attrs = [egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE];
    match egl.create_context(display, config, None, &es3_attrs) {
        Ok(context) => Ok(context),
        Err(es3_err) => {
            log::warn!("[DIAG-IMPELLER] EGL ES3 context failed: {es3_err:?}; trying ES2");
            let es2_attrs = [egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE];
            egl.create_context(display, config, None, &es2_attrs)
                .map_err(|es2_err| {
                    anyhow::anyhow!(
                        "eglCreateContext failed es3={:?} es2={:?}",
                        es3_err,
                        es2_err
                    )
                })
        }
    }
}

#[cfg(target_os = "android")]
fn create_platform_context() -> anyhow::Result<Context> {
    let entry = unsafe { ash::Entry::load() }
        .map_err(|err| anyhow::anyhow!("Failed to load Vulkan entry: {:?}", err))?;
    let context = unsafe {
        Context::new_vulkan(false, move |vk_instance, vk_proc_name| {
            if vk_proc_name.is_null() {
                return std::ptr::null_mut();
            }
            let instance = ash::vk::Instance::from_raw(vk_instance as u64);
            let proc = entry.get_instance_proc_addr(instance, vk_proc_name);
            proc.map(|f| f as *const () as *mut c_void)
                .unwrap_or(std::ptr::null_mut())
        })
    }
    .map_err(|err| anyhow::anyhow!("Failed to create Impeller Vulkan context: {:?}", err))?;
    Ok(context)
}

#[cfg(target_os = "macos")]
fn create_platform_context() -> anyhow::Result<Context> {
    unsafe { Context::new_metal() }
        .map_err(|err| anyhow::anyhow!("Failed to create Impeller Metal context: {:?}", err))
}

#[cfg(all(not(target_os = "android"), not(target_os = "macos")))]
fn create_platform_context() -> anyhow::Result<Context> {
    Err(anyhow::anyhow!(
        "ImpellerRuntime is only wired for Android Vulkan and macOS Metal"
    ))
}

#[cfg(target_os = "android")]
fn create_android_swapchain(
    context: &Context,
    native_window: *mut c_void,
    width: u32,
    height: u32,
) -> anyhow::Result<VkSwapChain> {
    if native_window.is_null() {
        return Err(anyhow::anyhow!("Impeller got null ANativeWindow"));
    }
    configure_android_native_window(native_window, width, height)?;
    let vk_info = context
        .get_vulkan_info()
        .map_err(|err| anyhow::anyhow!("Impeller Vulkan info unavailable: {}", err))?;
    let entry = unsafe { ash::Entry::load() }
        .map_err(|err| anyhow::anyhow!("Failed to load Vulkan entry: {:?}", err))?;
    let vk_instance_handle = ash::vk::Instance::from_raw(vk_info.vk_instance as u64);
    let instance = unsafe { ash::Instance::load(entry.static_fn(), vk_instance_handle) };
    let android_surface_fn = ash::khr::android_surface::Instance::new(&entry, &instance);
    let create_info =
        ash::vk::AndroidSurfaceCreateInfoKHR::default().window(native_window as *mut _);
    let vk_surface = unsafe { android_surface_fn.create_android_surface(&create_info, None) }
        .map_err(|err| anyhow::anyhow!("Failed to create Android Vulkan surface: {:?}", err))?;
    log_android_surface_capabilities(&entry, &instance, &vk_info, vk_surface);
    let swapchain =
        unsafe { context.create_new_vulkan_swapchain(vk_surface.as_raw() as *mut c_void) }
            .ok_or_else(|| anyhow::anyhow!("Failed to create Impeller Vulkan swapchain"))?;
    Ok(swapchain)
}

#[cfg(target_os = "android")]
fn configure_android_native_window(
    native_window: *mut c_void,
    width: u32,
    height: u32,
) -> anyhow::Result<()> {
    configure_android_native_window_with_format(native_window, width, height, 0)
}

#[cfg(target_os = "android")]
fn configure_android_native_window_with_format(
    native_window: *mut c_void,
    width: u32,
    height: u32,
    format: i32,
) -> anyhow::Result<()> {
    let window = native_window as *mut ndk_sys::ANativeWindow;
    let requested_w = width.max(1) as i32;
    let requested_h = height.max(1) as i32;
    let before_w = unsafe { ndk_sys::ANativeWindow_getWidth(window) };
    let before_h = unsafe { ndk_sys::ANativeWindow_getHeight(window) };
    let before_format = unsafe { ndk_sys::ANativeWindow_getFormat(window) };
    let rc = unsafe {
        // Vulkan keeps the Java SurfaceHolder/native format unchanged
        // (format=0). GLES passes EGL_NATIVE_VISUAL_ID so the ANativeWindow
        // buffer format matches the chosen EGLConfig.
        ndk_sys::ANativeWindow_setBuffersGeometry(window, requested_w, requested_h, format)
    };
    let after_w = unsafe { ndk_sys::ANativeWindow_getWidth(window) };
    let after_h = unsafe { ndk_sys::ANativeWindow_getHeight(window) };
    let after_format = unsafe { ndk_sys::ANativeWindow_getFormat(window) };
    log::info!(
        "[DIAG-IMPELLER] ANativeWindow buffers geometry req={}x{} fmt_req={} before={}x{} fmt={} after={}x{} fmt={} rc={}",
        requested_w,
        requested_h,
        format,
        before_w,
        before_h,
        before_format,
        after_w,
        after_h,
        after_format,
        rc
    );
    if rc != 0 {
        return Err(anyhow::anyhow!(
            "ANativeWindow_setBuffersGeometry failed rc={}",
            rc
        ));
    }
    Ok(())
}

#[cfg(target_os = "android")]
fn log_android_surface_capabilities(
    entry: &ash::Entry,
    instance: &ash::Instance,
    vk_info: &impellers::VulkanInfo,
    surface: ash::vk::SurfaceKHR,
) {
    let surface_fn = ash::khr::surface::Instance::new(entry, instance);
    let physical_device = ash::vk::PhysicalDevice::from_raw(vk_info.vk_physical_device as u64);
    match unsafe { surface_fn.get_physical_device_surface_capabilities(physical_device, surface) } {
        Ok(caps) => {
            log::info!(
                "[DIAG-IMPELLER] Vulkan surface extent current={}x{} min={}x{} max={}x{} min_images={} max_images={} usage=0x{:x}",
                caps.current_extent.width,
                caps.current_extent.height,
                caps.min_image_extent.width,
                caps.min_image_extent.height,
                caps.max_image_extent.width,
                caps.max_image_extent.height,
                caps.min_image_count,
                caps.max_image_count,
                caps.supported_usage_flags.as_raw()
            );
        }
        Err(err) => {
            log::warn!("[DIAG-IMPELLER] query Vulkan surface capabilities failed: {err:?}");
        }
    }
}

#[cfg(not(target_os = "android"))]
fn create_android_swapchain(
    _context: &Context,
    _native_window: *mut c_void,
    _width: u32,
    _height: u32,
) -> anyhow::Result<VkSwapChain> {
    Err(anyhow::anyhow!(
        "Android Impeller swapchain is only available on Android"
    ))
}
