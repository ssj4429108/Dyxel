// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, WindowHandle};
#[cfg(target_os = "android")] use raw_window_handle::AndroidNdkWindowHandle;
#[cfg(target_os = "ios")] use raw_window_handle::{UiKitDisplayHandle, UiKitWindowHandle};
#[cfg(target_os = "macos")] use raw_window_handle::{AppKitDisplayHandle, AppKitWindowHandle};
#[cfg(target_arch = "wasm32")] use raw_window_handle::{WebDisplayHandle, WebWindowHandle};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub u64);

pub struct SafeWindowHandle {
    #[cfg(target_os = "android")] android_window: Option<std::ptr::NonNull<ndk_sys::ANativeWindow>>,
    #[allow(dead_code)] raw_window_handle: RawWindowHandle,
    #[allow(dead_code)] raw_display_handle: RawDisplayHandle,
}

impl SafeWindowHandle {
    #[cfg(target_os = "android")]
    pub fn new_android(surface_ptr: u64) -> Self {
        let ptr = std::ptr::NonNull::new(surface_ptr as *mut ndk_sys::ANativeWindow).expect("Null");
        Self { 
            android_window: Some(ptr), 
            raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(ptr.cast())), 
            raw_display_handle: RawDisplayHandle::Android(raw_window_handle::AndroidDisplayHandle::new()) 
        }
    }
    #[cfg(target_os = "ios")]
    pub fn new_ios(surface_ptr: u64) -> Self {
        Self { 
            #[cfg(target_os = "android")] android_window: None, 
            raw_window_handle: RawWindowHandle::UiKit(raw_window_handle::UiKitWindowHandle::new(std::ptr::NonNull::new(surface_ptr as *mut _).unwrap())), 
            raw_display_handle: RawDisplayHandle::UiKit(raw_window_handle::UiKitDisplayHandle::new()) 
        }
    }

    #[cfg(target_os = "macos")]
    pub fn new_macos(surface_ptr: u64) -> Self {
        Self { 
            #[cfg(target_os = "android")] android_window: None, 
            raw_window_handle: RawWindowHandle::AppKit(AppKitWindowHandle::new(std::ptr::NonNull::new(surface_ptr as *mut _).unwrap())), 
            raw_display_handle: RawDisplayHandle::AppKit(AppKitDisplayHandle::new()) 
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn new_web(id: u32) -> Self {
        Self { 
            raw_window_handle: RawWindowHandle::Web(WebWindowHandle::new(id)), 
            raw_display_handle: RawDisplayHandle::Web(WebDisplayHandle::new()) 
        }
    }
}

#[cfg(target_os = "android")]
impl Drop for SafeWindowHandle { 
    fn drop(&mut self) { 
        if let Some(ptr) = self.android_window { 
            unsafe { ndk_sys::ANativeWindow_release(ptr.as_ptr()); } 
        } 
    } 
}

#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "C" fn Java_com_dyxel_android_DyxelEngine_getNativeSurface(
    env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jobject,
    surface: jni::sys::jobject,
) -> jni::sys::jlong {
    use ndk_sys::ANativeWindow_fromSurface;
    let window = ANativeWindow_fromSurface(env as *mut _, surface as *mut _);
    window as jni::sys::jlong
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_com_dyxel_android_DyxelEngine_initLogger(
    _env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jobject,
) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
}

impl HasWindowHandle for SafeWindowHandle { 
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> { 
        unsafe { Ok(WindowHandle::borrow_raw(self.raw_window_handle.clone())) } 
    } 
}

impl HasDisplayHandle for SafeWindowHandle { 
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> { 
        unsafe { Ok(DisplayHandle::borrow_raw(self.raw_display_handle.clone())) } 
    } 
}

unsafe impl Send for SafeWindowHandle {}
unsafe impl Sync for SafeWindowHandle {}

pub struct SurfaceState { 
    pub surface: vello::util::RenderSurface<'static>, 
    pub blit_pipeline: vello::wgpu::RenderPipeline, 
    pub offscreen_texture: Option<(vello::wgpu::Texture, vello::wgpu::BindGroup)>, 
    #[allow(dead_code)] pub window_handle: Option<Arc<SafeWindowHandle>> 
}

unsafe impl Send for SurfaceState {}
unsafe impl Sync for SurfaceState {}
