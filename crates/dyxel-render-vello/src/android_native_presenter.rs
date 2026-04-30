// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Android native-presenter experiment.
//!
//! This module is intentionally **not** part of the default render path.  It is
//! a Flutter/Impeller-inspired scaffolding for the path we need to investigate
//! next:
//!
//! ```text
//! AHardwareBuffer ring -> explicit acquire fence fd -> ASurfaceTransaction
//! ```
//!
//! The current production path still uses `wgpu::Surface`. This module keeps
//! the native-presenter scaffolding and diagnostics behind debug properties.
//!
//! Keep the CPU producer path separate from the Vulkan producer path.  A future
//! `vello-cpu` fallback can write RGBA8 pixels into the AHB ring via
//! `AHardwareBuffer_lock/unlock` without requiring Vulkan AHB import or
//! semaphore-fd export support.

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::NonNull;

const MIN_SURFACE_CONTROL_API: i32 = 29;
const DEFAULT_RING_DEPTH: usize = 3;
const NO_FENCE: i32 = -1;
const RGBA8_BYTES_PER_PIXEL: usize = 4;

const VK_EXT_AHB: &[u8] = b"VK_ANDROID_external_memory_android_hardware_buffer\0";
const VK_EXT_EXTERNAL_MEMORY: &[u8] = b"VK_KHR_external_memory\0";
const VK_EXT_EXTERNAL_MEMORY_FD: &[u8] = b"VK_KHR_external_memory_fd\0";
const VK_EXT_EXTERNAL_SEMAPHORE: &[u8] = b"VK_KHR_external_semaphore\0";
const VK_EXT_EXTERNAL_SEMAPHORE_FD: &[u8] = b"VK_KHR_external_semaphore_fd\0";
const VK_EXT_DEDICATED_ALLOCATION: &[u8] = b"VK_KHR_dedicated_allocation\0";
const VK_EXT_GET_MEMORY_REQUIREMENTS2: &[u8] = b"VK_KHR_get_memory_requirements2\0";
const VK_EXT_BIND_MEMORY2: &[u8] = b"VK_KHR_bind_memory2\0";
const VK_EXT_QUEUE_FAMILY_FOREIGN: &[u8] = b"VK_EXT_queue_family_foreign\0";
const VK_EXT_SAMPLER_YCBCR_CONVERSION: &[u8] = b"VK_KHR_sampler_ycbcr_conversion\0";

#[repr(C)]
struct ASurfaceControl {
    _private: [u8; 0],
}

#[repr(C)]
struct ASurfaceTransaction {
    _private: [u8; 0],
}

type SurfaceControlCreateFromWindowFn =
    unsafe extern "C" fn(*mut ndk_sys::ANativeWindow, *const c_char) -> *mut ASurfaceControl;
type SurfaceControlReleaseFn = unsafe extern "C" fn(*mut ASurfaceControl);
type SurfaceTransactionCreateFn = unsafe extern "C" fn() -> *mut ASurfaceTransaction;
type SurfaceTransactionDeleteFn = unsafe extern "C" fn(*mut ASurfaceTransaction);
type SurfaceTransactionApplyFn = unsafe extern "C" fn(*mut ASurfaceTransaction);
type SurfaceTransactionSetVisibilityFn =
    unsafe extern "C" fn(*mut ASurfaceTransaction, *mut ASurfaceControl, i8);
type SurfaceTransactionSetZOrderFn =
    unsafe extern "C" fn(*mut ASurfaceTransaction, *mut ASurfaceControl, i32);
type SurfaceTransactionSetBufferFn = unsafe extern "C" fn(
    *mut ASurfaceTransaction,
    *mut ASurfaceControl,
    *mut ndk_sys::AHardwareBuffer,
    i32,
);
type SurfaceTransactionSetGeometryFn = unsafe extern "C" fn(
    *mut ASurfaceTransaction,
    *mut ASurfaceControl,
    *const ndk_sys::ARect,
    *const ndk_sys::ARect,
    i32,
);

type AHardwareBufferAllocateFn = unsafe extern "C" fn(
    *const ndk_sys::AHardwareBuffer_Desc,
    *mut *mut ndk_sys::AHardwareBuffer,
) -> i32;
type AHardwareBufferReleaseFn = unsafe extern "C" fn(*mut ndk_sys::AHardwareBuffer);
type AHardwareBufferDescribeFn =
    unsafe extern "C" fn(*const ndk_sys::AHardwareBuffer, *mut ndk_sys::AHardwareBuffer_Desc);
type AHardwareBufferLockFn = unsafe extern "C" fn(
    *mut ndk_sys::AHardwareBuffer,
    u64,
    i32,
    *const ndk_sys::ARect,
    *mut *mut c_void,
) -> i32;
type AHardwareBufferUnlockFn = unsafe extern "C" fn(*mut ndk_sys::AHardwareBuffer, *mut i32) -> i32;

#[derive(Clone, Copy)]
struct AndroidNativeFns {
    create_from_window: SurfaceControlCreateFromWindowFn,
    release_surface_control: SurfaceControlReleaseFn,
    transaction_create: SurfaceTransactionCreateFn,
    transaction_delete: SurfaceTransactionDeleteFn,
    transaction_apply: SurfaceTransactionApplyFn,
    transaction_set_visibility: SurfaceTransactionSetVisibilityFn,
    transaction_set_z_order: Option<SurfaceTransactionSetZOrderFn>,
    transaction_set_buffer: SurfaceTransactionSetBufferFn,
    transaction_set_geometry: Option<SurfaceTransactionSetGeometryFn>,
    ahb_allocate: AHardwareBufferAllocateFn,
    ahb_release: AHardwareBufferReleaseFn,
    ahb_describe: AHardwareBufferDescribeFn,
    ahb_lock: AHardwareBufferLockFn,
    ahb_unlock: AHardwareBufferUnlockFn,
}

struct AndroidNativeApi {
    _libandroid: libloading::Library,
    fns: AndroidNativeFns,
}

impl AndroidNativeApi {
    unsafe fn load() -> anyhow::Result<Self> {
        let libandroid = unsafe { libloading::Library::new("libandroid.so") }
            .map_err(|e| anyhow::anyhow!("load libandroid.so failed: {e}"))?;

        let fns = AndroidNativeFns {
            create_from_window: load_fn(&libandroid, b"ASurfaceControl_createFromWindow\0")?,
            release_surface_control: load_fn(&libandroid, b"ASurfaceControl_release\0")?,
            transaction_create: load_fn(&libandroid, b"ASurfaceTransaction_create\0")?,
            transaction_delete: load_fn(&libandroid, b"ASurfaceTransaction_delete\0")?,
            transaction_apply: load_fn(&libandroid, b"ASurfaceTransaction_apply\0")?,
            transaction_set_visibility: load_fn(
                &libandroid,
                b"ASurfaceTransaction_setVisibility\0",
            )?,
            transaction_set_z_order: load_optional_fn(
                &libandroid,
                b"ASurfaceTransaction_setZOrder\0",
            ),
            transaction_set_buffer: load_fn(&libandroid, b"ASurfaceTransaction_setBuffer\0")?,
            transaction_set_geometry: load_optional_fn(
                &libandroid,
                b"ASurfaceTransaction_setGeometry\0",
            ),
            ahb_allocate: load_fn(&libandroid, b"AHardwareBuffer_allocate\0")?,
            ahb_release: load_fn(&libandroid, b"AHardwareBuffer_release\0")?,
            ahb_describe: load_fn(&libandroid, b"AHardwareBuffer_describe\0")?,
            ahb_lock: load_fn(&libandroid, b"AHardwareBuffer_lock\0")?,
            ahb_unlock: load_fn(&libandroid, b"AHardwareBuffer_unlock\0")?,
        };

        Ok(Self {
            _libandroid: libandroid,
            fns,
        })
    }
}

unsafe fn load_fn<T: Copy>(lib: &libloading::Library, symbol: &'static [u8]) -> anyhow::Result<T> {
    let loaded = unsafe { lib.get::<T>(symbol) }.map_err(|e| {
        let name = CStr::from_bytes_with_nul(symbol)
            .ok()
            .and_then(|s| s.to_str().ok())
            .unwrap_or("<invalid-symbol>");
        anyhow::anyhow!("load {name} failed: {e}")
    })?;
    Ok(*loaded)
}

fn load_optional_fn<T: Copy>(lib: &libloading::Library, symbol: &'static [u8]) -> Option<T> {
    match unsafe { lib.get::<T>(symbol) } {
        Ok(loaded) => Some(*loaded),
        Err(err) => {
            let name = CStr::from_bytes_with_nul(symbol)
                .ok()
                .and_then(|s| s.to_str().ok())
                .unwrap_or("<invalid-symbol>");
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] optional symbol {} unavailable: {}",
                name,
                err
            );
            None
        }
    }
}

struct NativeTransaction {
    raw: NonNull<ASurfaceTransaction>,
    fns: AndroidNativeFns,
}

impl NativeTransaction {
    fn new(fns: AndroidNativeFns) -> anyhow::Result<Self> {
        let raw = unsafe { (fns.transaction_create)() };
        Ok(Self {
            raw: NonNull::new(raw)
                .ok_or_else(|| anyhow::anyhow!("ASurfaceTransaction_create returned null"))?,
            fns,
        })
    }

    fn raw(&self) -> *mut ASurfaceTransaction {
        self.raw.as_ptr()
    }

    fn apply(self) {
        unsafe { (self.fns.transaction_apply)(self.raw()) };
    }
}

impl Drop for NativeTransaction {
    fn drop(&mut self) {
        unsafe { (self.fns.transaction_delete)(self.raw()) };
    }
}

pub(super) struct AndroidHardwareBufferSlot {
    pub(super) buffer: NonNull<ndk_sys::AHardwareBuffer>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) stride: u32,
}

struct ScopedAndroidHardwareBufferSlot {
    fns: AndroidNativeFns,
    slot: Option<AndroidHardwareBufferSlot>,
}

enum CpuAhbWrite<'a> {
    SolidRgba([u8; 4]),
    Rgba8888 {
        pixels: &'a [u8],
        width: u32,
        height: u32,
        stride_bytes: usize,
    },
}

impl ScopedAndroidHardwareBufferSlot {
    fn allocate(fns: AndroidNativeFns, width: u32, height: u32) -> anyhow::Result<Self> {
        Ok(Self {
            fns,
            slot: Some(AndroidHardwareBufferSlot::allocate(fns, width, height)?),
        })
    }

    fn slot(&self) -> &AndroidHardwareBufferSlot {
        self.slot
            .as_ref()
            .expect("scoped AHardwareBuffer slot already released")
    }
}

impl Drop for ScopedAndroidHardwareBufferSlot {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            unsafe { (self.fns.ahb_release)(slot.buffer.as_ptr()) };
        }
    }
}

impl AndroidHardwareBufferSlot {
    fn allocate(fns: AndroidNativeFns, width: u32, height: u32) -> anyhow::Result<Self> {
        let usage = ahb_usage_for_probe();
        let desc = ndk_sys::AHardwareBuffer_Desc {
            width,
            height,
            layers: 1,
            format: ndk_sys::AHardwareBuffer_Format::AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM.0,
            usage,
            stride: 0,
            rfu0: 0,
            rfu1: 0,
        };

        let mut raw = std::ptr::null_mut();
        let status = unsafe { (fns.ahb_allocate)(&desc, &mut raw) };
        if status != 0 {
            return Err(anyhow::anyhow!(
                "AHardwareBuffer_allocate failed status={} size={}x{} usage=0x{:x}",
                status,
                width,
                height,
                usage
            ));
        }
        let buffer = NonNull::new(raw)
            .ok_or_else(|| anyhow::anyhow!("AHardwareBuffer_allocate returned null"))?;

        let mut actual = ndk_sys::AHardwareBuffer_Desc {
            width: 0,
            height: 0,
            layers: 0,
            format: 0,
            usage: 0,
            stride: 0,
            rfu0: 0,
            rfu1: 0,
        };
        unsafe { (fns.ahb_describe)(buffer.as_ptr(), &mut actual) };

        Ok(Self {
            buffer,
            width: actual.width,
            height: actual.height,
            stride: actual.stride,
        })
    }

    fn fill_cpu_probe(&self, fns: AndroidNativeFns, rgba: [u8; 4]) -> anyhow::Result<i32> {
        self.write_cpu(fns, CpuAhbWrite::SolidRgba(rgba))
    }

    fn write_cpu_rgba8888(
        &self,
        fns: AndroidNativeFns,
        pixels: &[u8],
        width: u32,
        height: u32,
        stride_bytes: usize,
    ) -> anyhow::Result<i32> {
        self.write_cpu(
            fns,
            CpuAhbWrite::Rgba8888 {
                pixels,
                width,
                height,
                stride_bytes,
            },
        )
    }

    fn write_cpu(&self, fns: AndroidNativeFns, write: CpuAhbWrite<'_>) -> anyhow::Result<i32> {
        self.validate_cpu_write(&write)?;

        let mut addr: *mut c_void = std::ptr::null_mut();
        let lock_status = unsafe {
            (fns.ahb_lock)(
                self.buffer.as_ptr(),
                ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_RARELY.0
                    as u64,
                NO_FENCE,
                std::ptr::null(),
                &mut addr,
            )
        };
        if lock_status != 0 || addr.is_null() {
            return Err(anyhow::anyhow!(
                "AHardwareBuffer_lock failed status={} addr_null={}",
                lock_status,
                addr.is_null()
            ));
        }

        let row_pixels = self.stride.max(self.width) as usize;
        let slot_width = self.width as usize;
        let slot_height = self.height as usize;
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(addr as *mut u8, row_pixels * slot_height * 4)
        };
        match write {
            CpuAhbWrite::SolidRgba(rgba) => {
                for y in 0..slot_height {
                    let row = &mut bytes[y * row_pixels * RGBA8_BYTES_PER_PIXEL..]
                        [..slot_width * RGBA8_BYTES_PER_PIXEL];
                    for px in row.chunks_exact_mut(RGBA8_BYTES_PER_PIXEL) {
                        px.copy_from_slice(&rgba);
                    }
                }
            }
            CpuAhbWrite::Rgba8888 {
                pixels,
                width,
                height,
                stride_bytes,
            } => {
                let width = width as usize;
                let height = height as usize;
                let row_bytes = width * RGBA8_BYTES_PER_PIXEL;
                for y in 0..height {
                    let src = &pixels[y * stride_bytes..][..row_bytes];
                    let dst = &mut bytes[y * row_pixels * RGBA8_BYTES_PER_PIXEL..][..row_bytes];
                    dst.copy_from_slice(src);
                }
            }
        }

        let mut release_fence = NO_FENCE;
        let unlock_status = unsafe { (fns.ahb_unlock)(self.buffer.as_ptr(), &mut release_fence) };
        if unlock_status != 0 {
            if release_fence >= 0 {
                close_fd(release_fence);
            }
            return Err(anyhow::anyhow!(
                "AHardwareBuffer_unlock failed status={}",
                unlock_status
            ));
        }
        Ok(release_fence)
    }

    fn validate_cpu_write(&self, write: &CpuAhbWrite<'_>) -> anyhow::Result<()> {
        match write {
            CpuAhbWrite::SolidRgba(_) => Ok(()),
            CpuAhbWrite::Rgba8888 {
                pixels,
                width,
                height,
                stride_bytes,
            } => {
                let slot_width = self.width as usize;
                let slot_height = self.height as usize;
                let width = *width as usize;
                let height = *height as usize;
                if width > slot_width || height > slot_height {
                    return Err(anyhow::anyhow!(
                        "CPU frame {}x{} exceeds AHB slot {}x{}",
                        width,
                        height,
                        slot_width,
                        slot_height
                    ));
                }
                let row_bytes = width
                    .checked_mul(RGBA8_BYTES_PER_PIXEL)
                    .ok_or_else(|| anyhow::anyhow!("CPU frame row size overflow"))?;
                if *stride_bytes < row_bytes {
                    return Err(anyhow::anyhow!(
                        "CPU frame stride {} is smaller than row bytes {}",
                        stride_bytes,
                        row_bytes
                    ));
                }
                let required_len = if height == 0 {
                    0
                } else {
                    stride_bytes
                        .checked_mul(height - 1)
                        .and_then(|base| base.checked_add(row_bytes))
                        .ok_or_else(|| anyhow::anyhow!("CPU frame buffer size overflow"))?
                };
                if pixels.len() < required_len {
                    return Err(anyhow::anyhow!(
                        "CPU frame buffer too small len={} required={}",
                        pixels.len(),
                        required_len
                    ));
                }
                Ok(())
            }
        }
    }
}

fn ahb_usage_for_probe() -> u64 {
    ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE.0 as u64
        | ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_GPU_COLOR_OUTPUT.0 as u64
        | ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_COMPOSER_OVERLAY.0 as u64
        | ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_RARELY.0 as u64
}

pub(super) fn close_fd(fd: i32) {
    unsafe extern "C" {
        fn close(fd: i32) -> i32;
    }
    unsafe {
        let _ = close(fd);
    }
}

fn vk_ext_name(bytes: &'static [u8]) -> &'static CStr {
    CStr::from_bytes_with_nul(bytes).expect("valid Vulkan extension name")
}

fn vulkan_api_version_tuple(api_version: u32) -> (u32, u32, u32) {
    (
        api_version >> 22,
        (api_version >> 12) & 0x3ff,
        api_version & 0xfff,
    )
}

fn vulkan_api_at_least_1_1(api_version: u32) -> bool {
    let (major, minor, _) = vulkan_api_version_tuple(api_version);
    major > 1 || (major == 1 && minor >= 1)
}

#[derive(Clone, Copy, Debug, Default)]
struct VulkanNativePresenterExtensionSupport {
    android_hardware_buffer: bool,
    external_memory: bool,
    external_memory_fd: bool,
    external_semaphore: bool,
    external_semaphore_fd: bool,
    dedicated_allocation: bool,
    get_memory_requirements2: bool,
    bind_memory2: bool,
    queue_family_foreign: bool,
    sampler_ycbcr_conversion: bool,
}

impl VulkanNativePresenterExtensionSupport {
    fn from_support_predicate(
        api_version: u32,
        mut supports: impl FnMut(&'static [u8]) -> bool,
    ) -> Self {
        let mut support = Self {
            android_hardware_buffer: supports(VK_EXT_AHB),
            external_memory: supports(VK_EXT_EXTERNAL_MEMORY),
            external_memory_fd: supports(VK_EXT_EXTERNAL_MEMORY_FD),
            external_semaphore: supports(VK_EXT_EXTERNAL_SEMAPHORE),
            external_semaphore_fd: supports(VK_EXT_EXTERNAL_SEMAPHORE_FD),
            dedicated_allocation: supports(VK_EXT_DEDICATED_ALLOCATION),
            get_memory_requirements2: supports(VK_EXT_GET_MEMORY_REQUIREMENTS2),
            bind_memory2: supports(VK_EXT_BIND_MEMORY2),
            queue_family_foreign: supports(VK_EXT_QUEUE_FAMILY_FOREIGN),
            sampler_ycbcr_conversion: supports(VK_EXT_SAMPLER_YCBCR_CONVERSION),
        };
        support.apply_core_promotions(api_version);
        support
    }

    fn from_enabled_extensions(enabled: &[&'static CStr], api_version: u32) -> Self {
        Self::from_support_predicate(api_version, |name| {
            let name = vk_ext_name(name);
            enabled.iter().any(|enabled_name| *enabled_name == name)
        })
    }

    fn apply_core_promotions(&mut self, api_version: u32) {
        // Vulkan 1.1 promoted the cross-platform external memory/semaphore and
        // memory-requirement helpers to core.  The Android AHB and fd-handle
        // extensions still need explicit extension names.
        if vulkan_api_at_least_1_1(api_version) {
            self.external_memory = true;
            self.external_semaphore = true;
            self.dedicated_allocation = true;
            self.get_memory_requirements2 = true;
            self.bind_memory2 = true;
            self.sampler_ycbcr_conversion = true;
        }
    }

    fn required_missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.android_hardware_buffer {
            missing.push("VK_ANDROID_external_memory_android_hardware_buffer");
        }
        if !self.external_memory {
            missing.push("VK_KHR_external_memory/core1.1");
        }
        if !self.external_semaphore {
            missing.push("VK_KHR_external_semaphore/core1.1");
        }
        if !self.external_semaphore_fd {
            missing.push("VK_KHR_external_semaphore_fd");
        }
        missing
    }

    fn advisory_missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.dedicated_allocation {
            missing.push("VK_KHR_dedicated_allocation/core1.1");
        }
        if !self.get_memory_requirements2 {
            missing.push("VK_KHR_get_memory_requirements2/core1.1");
        }
        if !self.bind_memory2 {
            missing.push("VK_KHR_bind_memory2/core1.1");
        }
        if !self.queue_family_foreign {
            missing.push("VK_EXT_queue_family_foreign");
        }
        if !self.sampler_ycbcr_conversion {
            missing.push("VK_KHR_sampler_ycbcr_conversion/core1.1");
        }
        missing
    }

    fn describe(&self) -> String {
        format!(
            "AHB={} external_memory={} external_memory_fd={} external_semaphore={} external_semaphore_fd={} dedicated_allocation={} get_memory_requirements2={} bind_memory2={} queue_family_foreign={} sampler_ycbcr_conversion={}",
            self.android_hardware_buffer,
            self.external_memory,
            self.external_memory_fd,
            self.external_semaphore,
            self.external_semaphore_fd,
            self.dedicated_allocation,
            self.get_memory_requirements2,
            self.bind_memory2,
            self.queue_family_foreign,
            self.sampler_ycbcr_conversion
        )
    }
}

fn android_property(name: &str) -> Option<String> {
    const PROP_VALUE_MAX: usize = 92;
    unsafe extern "C" {
        fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
    }

    let name = CString::new(name).ok()?;
    let mut value = [0 as c_char; PROP_VALUE_MAX];
    let len = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return None;
    }
    unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_str()
        .ok()
        .map(|s| s.to_string())
}

fn setting_value(env_name: &str, property_name: &str) -> Option<String> {
    std::env::var(env_name)
        .ok()
        .or_else(|| android_property(property_name))
}

fn setting_flag(env_name: &str, property_name: &str, default: bool) -> bool {
    setting_value(env_name, property_name)
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO" | ""))
        .unwrap_or(default)
}

pub(crate) fn android_native_presenter_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER",
        "debug.dyxel.native_presenter",
        false,
    )
}

pub(crate) fn android_native_presenter_diag_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER_DIAG",
        "debug.dyxel.native_presenter_diag",
        false,
    )
}

pub(crate) fn android_native_presenter_custom_device_probe_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER_CUSTOM_DEVICE_PROBE",
        "debug.dyxel.native_presenter_custom_device_probe",
        false,
    )
}

pub(crate) fn android_native_presenter_custom_device_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER_CUSTOM_DEVICE",
        "debug.dyxel.native_presenter_custom_device",
        false,
    )
}

pub(crate) fn android_native_presenter_ahb_import_probe_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER_AHB_IMPORT_PROBE",
        "debug.dyxel.native_presenter_ahb_import_probe",
        false,
    )
}

pub(crate) fn android_native_presenter_gpu_clear_probe_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER_GPU_CLEAR_PROBE",
        "debug.dyxel.native_presenter_gpu_clear_probe",
        false,
    )
}

pub(crate) fn android_native_presenter_gpu_present_probe_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER_GPU_PRESENT_PROBE",
        "debug.dyxel.native_presenter_gpu_present_probe",
        false,
    )
}

fn android_native_presenter_probe_present_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER_PROBE_PRESENT",
        "debug.dyxel.native_presenter_probe_present",
        false,
    )
}

fn android_native_presenter_ring_depth() -> usize {
    setting_value(
        "DYXEL_ANDROID_NATIVE_PRESENTER_DEPTH",
        "debug.dyxel.native_presenter_depth",
    )
    .and_then(|v| v.parse::<usize>().ok())
    .unwrap_or(DEFAULT_RING_DEPTH)
    .clamp(2, 4)
}

pub(crate) fn log_android_cpu_ahb_presenter_support() {
    let api_level = unsafe { ndk_sys::android_get_device_api_level() };
    log::info!(
        "[DIAG-NATIVE-PRESENTER] CPU AHB presenter support: api={} surface_control_api_ok={} rgba8_lock_unlock=true explicit_unlock_fence=true vulkan_interop_required=false",
        api_level,
        api_level >= MIN_SURFACE_CONTROL_API
    );
}

pub(crate) struct AndroidNativePresenterProbe {
    api: AndroidNativeApi,
    surface_control: NonNull<ASurfaceControl>,
    pub(super) buffers: Vec<AndroidHardwareBufferSlot>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) next_cpu_slot: usize,
    pub(super) presented_cpu_frames: u64,
    submitted_probe: bool,
    submitted_gpu_probe: bool,
}

unsafe impl Send for AndroidNativePresenterProbe {}
unsafe impl Sync for AndroidNativePresenterProbe {}

impl AndroidNativePresenterProbe {
    pub(crate) fn new_from_anative_window_ptr(
        native_window_ptr: u64,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Self> {
        let api_level = unsafe { ndk_sys::android_get_device_api_level() };
        if api_level < MIN_SURFACE_CONTROL_API {
            return Err(anyhow::anyhow!(
                "Android native presenter requires API {}+ (device API={})",
                MIN_SURFACE_CONTROL_API,
                api_level
            ));
        }

        let native_window = NonNull::new(native_window_ptr as *mut ndk_sys::ANativeWindow)
            .ok_or_else(|| anyhow::anyhow!("native presenter got null ANativeWindow"))?;
        let api = unsafe { AndroidNativeApi::load()? };
        let name = CString::new("dyxel-native-presenter")
            .map_err(|_| anyhow::anyhow!("invalid SurfaceControl name"))?;
        let surface_control =
            unsafe { (api.fns.create_from_window)(native_window.as_ptr(), name.as_ptr()) };
        let surface_control = NonNull::new(surface_control)
            .ok_or_else(|| anyhow::anyhow!("ASurfaceControl_createFromWindow returned null"))?;

        let mut presenter = Self {
            api,
            surface_control,
            buffers: Vec::new(),
            width,
            height,
            next_cpu_slot: 0,
            presented_cpu_frames: 0,
            submitted_probe: false,
            submitted_gpu_probe: false,
        };
        presenter.allocate_ring(width, height)?;
        presenter.hide()?;

        log::info!(
            "[DIAG-NATIVE-PRESENTER] created hidden SurfaceControl AHB probe size={}x{} depth={} api={}",
            width,
            height,
            presenter.buffers.len(),
            api_level
        );
        log_android_cpu_ahb_presenter_support();

        if android_native_presenter_probe_present_enabled() {
            presenter.submit_cpu_probe_once()?;
        }

        Ok(presenter)
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) -> anyhow::Result<()> {
        if self.width == width && self.height == height {
            return Ok(());
        }
        self.buffers.clear();
        self.allocate_ring(width, height)?;
        self.width = width;
        self.height = height;
        self.next_cpu_slot = 0;
        self.submitted_probe = false;
        self.submitted_gpu_probe = false;
        self.hide()?;
        log::info!(
            "[DIAG-NATIVE-PRESENTER] resized hidden AHB probe size={}x{} depth={}",
            width,
            height,
            self.buffers.len()
        );
        if android_native_presenter_probe_present_enabled() {
            self.submit_cpu_probe_once()?;
        }
        Ok(())
    }

    fn allocate_ring(&mut self, width: u32, height: u32) -> anyhow::Result<()> {
        self.buffers.clear();
        let depth = android_native_presenter_ring_depth();
        self.buffers.reserve(depth);
        for _ in 0..depth {
            self.buffers.push(AndroidHardwareBufferSlot::allocate(
                self.api.fns,
                width,
                height,
            )?);
        }
        Ok(())
    }

    fn hide(&self) -> anyhow::Result<()> {
        let transaction = NativeTransaction::new(self.api.fns)?;
        unsafe {
            (self.api.fns.transaction_set_visibility)(
                transaction.raw(),
                self.surface_control.as_ptr(),
                0,
            );
            if let Some(set_z_order) = self.api.fns.transaction_set_z_order {
                set_z_order(transaction.raw(), self.surface_control.as_ptr(), 0);
            }
        }
        transaction.apply();
        Ok(())
    }

    pub(super) fn show_slot_with_buffer(
        &self,
        slot: &AndroidHardwareBufferSlot,
        acquire_fence_fd: i32,
    ) -> anyhow::Result<()> {
        let transaction = match NativeTransaction::new(self.api.fns) {
            Ok(transaction) => transaction,
            Err(err) => {
                if acquire_fence_fd >= 0 {
                    close_fd(acquire_fence_fd);
                }
                return Err(err);
            }
        };
        unsafe {
            if let Some(set_geometry) = self.api.fns.transaction_set_geometry {
                let rect = ndk_sys::ARect {
                    left: 0,
                    top: 0,
                    right: slot.width as i32,
                    bottom: slot.height as i32,
                };
                set_geometry(
                    transaction.raw(),
                    self.surface_control.as_ptr(),
                    &rect,
                    &rect,
                    0,
                );
            }
            (self.api.fns.transaction_set_buffer)(
                transaction.raw(),
                self.surface_control.as_ptr(),
                slot.buffer.as_ptr(),
                acquire_fence_fd,
            );
            if let Some(set_z_order) = self.api.fns.transaction_set_z_order {
                set_z_order(transaction.raw(), self.surface_control.as_ptr(), 1);
            }
            (self.api.fns.transaction_set_visibility)(
                transaction.raw(),
                self.surface_control.as_ptr(),
                1,
            );
        }
        transaction.apply();
        Ok(())
    }

    /// Present one CPU-produced RGBA8 frame through the AHB ring.
    #[allow(dead_code)]
    pub(crate) fn present_cpu_rgba8888_frame(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
        stride_bytes: usize,
    ) -> anyhow::Result<()> {
        if width != self.width || height != self.height {
            return Err(anyhow::anyhow!(
                "CPU AHB presenter frame size {}x{} does not match current surface {}x{}",
                width,
                height,
                self.width,
                self.height
            ));
        }
        let slot_index = self.next_cpu_slot % self.buffers.len().max(1);
        let slot = self
            .buffers
            .get(slot_index)
            .ok_or_else(|| anyhow::anyhow!("native presenter has no AHB buffers"))?;
        let acquire_fence_fd =
            slot.write_cpu_rgba8888(self.api.fns, pixels, width, height, stride_bytes)?;
        self.show_slot_with_buffer(slot, acquire_fence_fd)?;
        self.next_cpu_slot = (slot_index + 1) % self.buffers.len().max(1);
        self.presented_cpu_frames = self.presented_cpu_frames.saturating_add(1);
        if self.presented_cpu_frames == 1 || self.presented_cpu_frames % 120 == 0 {
            log::info!(
                "[DIAG-NATIVE-PRESENTER] presented CPU RGBA8 AHB frame count={} slot={} size={}x{} stride_bytes={}",
                self.presented_cpu_frames,
                slot_index,
                width,
                height,
                stride_bytes
            );
        }
        // ASurfaceTransaction_setBuffer takes acquire_fence_fd ownership.
        Ok(())
    }

    fn submit_cpu_probe_once(&mut self) -> anyhow::Result<()> {
        if self.submitted_probe {
            return Ok(());
        }
        let slot = self
            .buffers
            .first()
            .ok_or_else(|| anyhow::anyhow!("native presenter has no AHB buffers"))?;
        let acquire_fence_fd = slot.fill_cpu_probe(self.api.fns, [0x20, 0x60, 0xff, 0xff])?;
        self.show_slot_with_buffer(slot, acquire_fence_fd)?;
        self.submitted_probe = true;
        log::warn!(
            "[DIAG-NATIVE-PRESENTER] submitted visible CPU AHB probe buffer; this is an opt-in visual override"
        );
        // ASurfaceTransaction_setBuffer takes acquire_fence_fd ownership.
        Ok(())
    }

    pub(crate) fn submit_gpu_clear_probe_once(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
    ) -> anyhow::Result<()> {
        if self.submitted_gpu_probe {
            return Ok(());
        }
        let slot_index = 0;
        let slot = self
            .buffers
            .get(slot_index)
            .ok_or_else(|| anyhow::anyhow!("native presenter has no AHB buffers"))?;
        let acquire_fence_fd =
            clear_ahb_slot_with_vulkan_and_export_sync_fd(adapter, device, slot)?;
        self.show_slot_with_buffer(slot, acquire_fence_fd)?;
        self.submitted_gpu_probe = true;
        log::warn!(
            "[DIAG-NATIVE-PRESENTER] submitted visible GPU-filled AHB SurfaceControl probe; this is an opt-in visual override"
        );
        // ASurfaceTransaction_setBuffer takes acquire_fence_fd ownership.
        Ok(())
    }

    pub(crate) fn submit_wgpu_ahb_texture_probe_once(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> anyhow::Result<()> {
        if self.submitted_gpu_probe {
            return Ok(());
        }
        let slot = self
            .buffers
            .first()
            .ok_or_else(|| anyhow::anyhow!("native presenter has no AHB buffers"))?;
        let acquire_fence_fd =
            super::android_native_wgpu_ahb::clear_slot_with_wgpu_texture_and_export_sync_fd(
                adapter, device, queue, slot,
            )?;
        self.show_slot_with_buffer(slot, acquire_fence_fd)?;
        self.submitted_gpu_probe = true;
        log::warn!(
            "[DIAG-NATIVE-PRESENTER] submitted visible wgpu-written AHB SurfaceControl probe; this is an opt-in visual override"
        );
        Ok(())
    }
}

impl Drop for AndroidNativePresenterProbe {
    fn drop(&mut self) {
        for slot in self.buffers.drain(..) {
            unsafe { (self.api.fns.ahb_release)(slot.buffer.as_ptr()) };
        }
        unsafe { (self.api.fns.release_surface_control)(self.surface_control.as_ptr()) };
        log::info!("[DIAG-NATIVE-PRESENTER] released SurfaceControl AHB probe");
    }
}

pub(crate) fn log_wgpu_vulkan_external_ahb_support(adapter: &wgpu::Adapter, device: &wgpu::Device) {
    let Some(vk_device) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        log::warn!(
            "[DIAG-NATIVE-PRESENTER] current wgpu backend is not Vulkan; AHB import path unavailable"
        );
        return;
    };
    let Some(vk_adapter) = (unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        log::warn!(
            "[DIAG-NATIVE-PRESENTER] current wgpu adapter is not Vulkan; AHB import path unavailable"
        );
        return;
    };

    let caps = vk_adapter.physical_device_capabilities();
    let api_version = caps.properties().api_version;
    let (api_major, api_minor, api_patch) = vulkan_api_version_tuple(api_version);
    let available =
        VulkanNativePresenterExtensionSupport::from_support_predicate(api_version, |name| {
            caps.supports_extension(vk_ext_name(name))
        });
    let enabled = VulkanNativePresenterExtensionSupport::from_enabled_extensions(
        vk_device.enabled_device_extensions(),
        api_version,
    );
    let available_missing = available.required_missing();
    let enabled_missing = enabled.required_missing();
    let advisory_missing = available.advisory_missing();

    log::info!(
        "[DIAG-NATIVE-PRESENTER] Vulkan physical device api={}.{}.{} available: {}",
        api_major,
        api_minor,
        api_patch,
        available.describe()
    );
    log::info!(
        "[DIAG-NATIVE-PRESENTER] wgpu Vulkan device effective enabled/core: {} queue_family={}",
        enabled.describe(),
        vk_device.queue_family_index()
    );
    if !available_missing.is_empty() {
        log::warn!(
            "[DIAG-NATIVE-PRESENTER] native GPU presenter blocked by driver/physical-device support: missing {}",
            available_missing.join(", ")
        );
    } else if !enabled_missing.is_empty() {
        log::warn!(
            "[DIAG-NATIVE-PRESENTER] native GPU presenter blocked by current wgpu Device creation: driver exposes the minimum interop set, but wgpu did not enable {}",
            enabled_missing.join(", ")
        );
        log::warn!(
            "[DIAG-NATIVE-PRESENTER] next implementation step needs a native Vulkan device/presenter or a wgpu-hal open_with_callback/create_device_from_hal path that enables AHB + external semaphore fd before Device creation"
        );
    } else {
        log::info!(
            "[DIAG-NATIVE-PRESENTER] native GPU presenter minimum AHB + semaphore-fd interop is enabled on the current wgpu device"
        );
    }
    if !advisory_missing.is_empty() {
        log::warn!(
            "[DIAG-NATIVE-PRESENTER] advisory Vulkan interop helpers not reported available: {}",
            advisory_missing.join(", ")
        );
    }
}

pub(crate) fn probe_wgpu_vulkan_ahb_import(adapter: &wgpu::Adapter, device: &wgpu::Device) {
    match wgpu_vulkan_device_native_interop_enabled(device) {
        Ok(true) => {
            match run_vulkan_ahb_import_probe_on_wgpu_device(device, "current-wgpu-device") {
                Ok(()) => log::info!(
                    "[DIAG-NATIVE-PRESENTER] Vulkan AHB import probe succeeded on current wgpu Device"
                ),
                Err(err) => log::warn!(
                    "[DIAG-NATIVE-PRESENTER] Vulkan AHB import probe failed on current wgpu Device: {:?}",
                    err
                ),
            }
        }
        Ok(false) => {
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] current wgpu Device lacks enabled AHB/semaphore-fd interop; probing a temporary custom Vulkan Device instead"
            );
            probe_wgpu_custom_vulkan_device_ahb_import(adapter);
        }
        Err(err) => {
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] Vulkan AHB import probe unavailable on current wgpu Device: {:?}",
                err
            );
        }
    }
}

pub(crate) fn probe_wgpu_vulkan_ahb_gpu_clear(adapter: &wgpu::Adapter, device: &wgpu::Device) {
    match wgpu_vulkan_device_native_interop_enabled(device) {
        Ok(true) => {
            match run_vulkan_ahb_gpu_clear_probe_on_wgpu_device(device, "current-wgpu-device") {
                Ok(()) => log::info!(
                    "[DIAG-NATIVE-PRESENTER] Vulkan AHB GPU clear/sync-fd probe succeeded on current wgpu Device"
                ),
                Err(err) => log::warn!(
                    "[DIAG-NATIVE-PRESENTER] Vulkan AHB GPU clear/sync-fd probe failed on current wgpu Device: {:?}",
                    err
                ),
            }
        }
        Ok(false) => {
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] current wgpu Device lacks enabled AHB/semaphore-fd interop; GPU clear/sync-fd probing a temporary custom Vulkan Device instead"
            );
            probe_wgpu_custom_vulkan_device_ahb_gpu_clear(adapter);
        }
        Err(err) => {
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] Vulkan AHB GPU clear/sync-fd probe unavailable on current wgpu Device: {:?}",
                err
            );
        }
    }
}

fn clear_ahb_slot_with_vulkan_and_export_sync_fd(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<i32> {
    match wgpu_vulkan_device_native_interop_enabled(device) {
        Ok(true) => clear_ahb_slot_on_wgpu_device_and_export_sync_fd(
            device,
            "current-wgpu-device-present-probe",
            slot,
        ),
        Ok(false) => {
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] current wgpu Device lacks enabled AHB/semaphore-fd interop; GPU SurfaceControl present probe using a temporary custom Vulkan Device"
            );
            clear_ahb_slot_with_temp_custom_vulkan_device(adapter, slot)
        }
        Err(err) => Err(anyhow::anyhow!(
            "Vulkan AHB GPU present probe unavailable on current wgpu Device: {:?}",
            err
        )),
    }
}

fn clear_ahb_slot_with_temp_custom_vulkan_device(
    adapter: &wgpu::Adapter,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<i32> {
    let features = default_native_presenter_wgpu_features(adapter);
    let (device, queue) = create_wgpu_custom_vulkan_device_with_android_interop(
        adapter,
        features,
        "Dyxel Android native presenter AHB GPU present probe Vulkan device",
    )?;
    log_wgpu_vulkan_external_ahb_support(adapter, &device);
    let result = clear_ahb_slot_on_wgpu_device_and_export_sync_fd(
        &device,
        "custom-present-probe-device",
        slot,
    );
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    drop(queue);
    drop(device);
    result
}

fn probe_wgpu_custom_vulkan_device_ahb_import(adapter: &wgpu::Adapter) {
    let features = default_native_presenter_wgpu_features(adapter);
    let (device, queue) = match create_wgpu_custom_vulkan_device_with_android_interop(
        adapter,
        features,
        "Dyxel Android native presenter AHB import probe Vulkan device",
    ) {
        Ok(created) => created,
        Err(err) => {
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] custom Vulkan AHB import probe Device creation failed: {:?}",
                err
            );
            return;
        }
    };

    log_wgpu_vulkan_external_ahb_support(adapter, &device);
    match run_vulkan_ahb_import_probe_on_wgpu_device(&device, "custom-probe-device") {
        Ok(()) => log::info!(
            "[DIAG-NATIVE-PRESENTER] Vulkan AHB import probe succeeded on temporary custom Device"
        ),
        Err(err) => log::warn!(
            "[DIAG-NATIVE-PRESENTER] Vulkan AHB import probe failed on temporary custom Device: {:?}",
            err
        ),
    }
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    drop(queue);
    drop(device);
}

fn probe_wgpu_custom_vulkan_device_ahb_gpu_clear(adapter: &wgpu::Adapter) {
    let features = default_native_presenter_wgpu_features(adapter);
    let (device, queue) = match create_wgpu_custom_vulkan_device_with_android_interop(
        adapter,
        features,
        "Dyxel Android native presenter AHB GPU clear probe Vulkan device",
    ) {
        Ok(created) => created,
        Err(err) => {
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] custom Vulkan AHB GPU clear probe Device creation failed: {:?}",
                err
            );
            return;
        }
    };

    log_wgpu_vulkan_external_ahb_support(adapter, &device);
    match run_vulkan_ahb_gpu_clear_probe_on_wgpu_device(&device, "custom-probe-device") {
        Ok(()) => log::info!(
            "[DIAG-NATIVE-PRESENTER] Vulkan AHB GPU clear/sync-fd probe succeeded on temporary custom Device"
        ),
        Err(err) => log::warn!(
            "[DIAG-NATIVE-PRESENTER] Vulkan AHB GPU clear/sync-fd probe failed on temporary custom Device: {:?}",
            err
        ),
    }
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    drop(queue);
    drop(device);
}

pub(super) fn wgpu_vulkan_device_native_interop_enabled(
    device: &wgpu::Device,
) -> anyhow::Result<bool> {
    let Some(vk_device) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        return Err(anyhow::anyhow!("current wgpu backend is not Vulkan"));
    };
    let api_version = unsafe {
        vk_device
            .shared_instance()
            .raw_instance()
            .get_physical_device_properties(vk_device.raw_physical_device())
            .api_version
    };
    let enabled = VulkanNativePresenterExtensionSupport::from_enabled_extensions(
        vk_device.enabled_device_extensions(),
        api_version,
    );
    Ok(enabled.required_missing().is_empty())
}

fn run_vulkan_ahb_import_probe_on_wgpu_device(
    device: &wgpu::Device,
    label: &'static str,
) -> anyhow::Result<()> {
    let Some(vk_device) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        return Err(anyhow::anyhow!("wgpu backend is not Vulkan"));
    };
    let api_version = unsafe {
        vk_device
            .shared_instance()
            .raw_instance()
            .get_physical_device_properties(vk_device.raw_physical_device())
            .api_version
    };
    let enabled = VulkanNativePresenterExtensionSupport::from_enabled_extensions(
        vk_device.enabled_device_extensions(),
        api_version,
    );
    let missing = enabled.required_missing();
    if !missing.is_empty() {
        return Err(anyhow::anyhow!(
            "wgpu Vulkan Device did not enable {}",
            missing.join(", ")
        ));
    }

    unsafe { run_vulkan_ahb_import_probe_raw(&vk_device, label) }
}

fn run_vulkan_ahb_gpu_clear_probe_on_wgpu_device(
    device: &wgpu::Device,
    label: &'static str,
) -> anyhow::Result<()> {
    let Some(vk_device) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        return Err(anyhow::anyhow!("wgpu backend is not Vulkan"));
    };
    let api_version = unsafe {
        vk_device
            .shared_instance()
            .raw_instance()
            .get_physical_device_properties(vk_device.raw_physical_device())
            .api_version
    };
    let enabled = VulkanNativePresenterExtensionSupport::from_enabled_extensions(
        vk_device.enabled_device_extensions(),
        api_version,
    );
    let missing = enabled.required_missing();
    if !missing.is_empty() {
        return Err(anyhow::anyhow!(
            "wgpu Vulkan Device did not enable {}",
            missing.join(", ")
        ));
    }
    if !enabled.queue_family_foreign {
        return Err(anyhow::anyhow!(
            "wgpu Vulkan Device did not enable VK_EXT_queue_family_foreign"
        ));
    }

    unsafe { run_vulkan_ahb_gpu_clear_probe_raw(&vk_device, label) }
}

fn clear_ahb_slot_on_wgpu_device_and_export_sync_fd(
    device: &wgpu::Device,
    label: &'static str,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<i32> {
    let Some(vk_device) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        return Err(anyhow::anyhow!("wgpu backend is not Vulkan"));
    };
    let api_version = unsafe {
        vk_device
            .shared_instance()
            .raw_instance()
            .get_physical_device_properties(vk_device.raw_physical_device())
            .api_version
    };
    let enabled = VulkanNativePresenterExtensionSupport::from_enabled_extensions(
        vk_device.enabled_device_extensions(),
        api_version,
    );
    let missing = enabled.required_missing();
    if !missing.is_empty() {
        return Err(anyhow::anyhow!(
            "wgpu Vulkan Device did not enable {}",
            missing.join(", ")
        ));
    }
    if !enabled.queue_family_foreign {
        return Err(anyhow::anyhow!(
            "wgpu Vulkan Device did not enable VK_EXT_queue_family_foreign"
        ));
    }

    unsafe { clear_ahb_slot_on_vulkan_device_and_export_sync_fd(&vk_device, label, slot) }
}

unsafe fn run_vulkan_ahb_import_probe_raw(
    vk_device: &wgpu::hal::vulkan::Device,
    label: &'static str,
) -> anyhow::Result<()> {
    let api = unsafe { AndroidNativeApi::load()? };
    // Keep the proof allocation deliberately tiny: this only validates the AHB
    // image/memory-import seam, not throughput or presentation.
    let ahb = ScopedAndroidHardwareBufferSlot::allocate(api.fns, 64, 64)?;
    let slot = ahb.slot();
    let raw_device = vk_device.raw_device();
    let imported = unsafe { import_ahb_slot_to_vulkan_image(vk_device, slot)? };
    let vk_format = imported.format;
    let format_features = imported.format_features;
    let allocation_size = imported.allocation_size;
    let memory_type_bits = imported.memory_type_bits;
    let memory_type_index = imported.memory_type_index;
    unsafe { destroy_imported_ahb_vk_image(raw_device, imported) };

    log::info!(
        "[DIAG-NATIVE-PRESENTER] Vulkan AHB import probe ok label={} size={}x{} stride={} vk_format={:?} format_features={:?} allocation_size={} memory_type_bits=0x{:x} memory_type_index={} queue_family={}",
        label,
        slot.width,
        slot.height,
        slot.stride,
        vk_format,
        format_features,
        allocation_size,
        memory_type_bits,
        memory_type_index,
        vk_device.queue_family_index()
    );
    Ok(())
}

unsafe fn run_vulkan_ahb_gpu_clear_probe_raw(
    vk_device: &wgpu::hal::vulkan::Device,
    label: &'static str,
) -> anyhow::Result<()> {
    let api = unsafe { AndroidNativeApi::load()? };
    // This is still diagnostic-only and deliberately tiny. It proves the next
    // seam after AHB import: Vulkan writes into the imported image, then exports
    // a sync fd that can become an ASurfaceTransaction acquire fence later.
    let ahb = ScopedAndroidHardwareBufferSlot::allocate(api.fns, 64, 64)?;
    let slot = ahb.slot();
    let raw_device = vk_device.raw_device();
    let imported = unsafe { import_ahb_slot_to_vulkan_image(vk_device, slot)? };
    let vk_format = imported.format;
    let format_features = imported.format_features;
    let allocation_size = imported.allocation_size;
    let memory_type_bits = imported.memory_type_bits;
    let memory_type_index = imported.memory_type_index;
    let exported_sync_fd = match unsafe {
        clear_imported_ahb_vk_image_and_export_sync_fd(vk_device, imported.image)
    } {
        Ok(fd) => fd,
        Err(err) => {
            unsafe { destroy_imported_ahb_vk_image(raw_device, imported) };
            return Err(err);
        }
    };
    unsafe { destroy_imported_ahb_vk_image(raw_device, imported) };

    // This probe does not hand the fd to SurfaceFlinger yet, so ownership stays
    // here and we must close it.  A ready sync fd may be represented as -1.
    if exported_sync_fd >= 0 {
        close_fd(exported_sync_fd);
    }

    log::info!(
        "[DIAG-NATIVE-PRESENTER] Vulkan AHB GPU clear/sync-fd probe ok label={} size={}x{} stride={} vk_format={:?} format_features={:?} allocation_size={} memory_type_bits=0x{:x} memory_type_index={} exported_sync_fd={} queue_family={}",
        label,
        slot.width,
        slot.height,
        slot.stride,
        vk_format,
        format_features,
        allocation_size,
        memory_type_bits,
        memory_type_index,
        exported_sync_fd,
        vk_device.queue_family_index()
    );
    Ok(())
}

unsafe fn clear_ahb_slot_on_vulkan_device_and_export_sync_fd(
    vk_device: &wgpu::hal::vulkan::Device,
    label: &'static str,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<i32> {
    let raw_device = vk_device.raw_device();
    let imported = unsafe { import_ahb_slot_to_vulkan_image(vk_device, slot)? };
    let vk_format = imported.format;
    let format_features = imported.format_features;
    let allocation_size = imported.allocation_size;
    let memory_type_bits = imported.memory_type_bits;
    let memory_type_index = imported.memory_type_index;
    let exported_sync_fd = match unsafe {
        clear_imported_ahb_vk_image_and_export_sync_fd(vk_device, imported.image)
    } {
        Ok(fd) => fd,
        Err(err) => {
            unsafe { destroy_imported_ahb_vk_image(raw_device, imported) };
            return Err(err);
        }
    };
    unsafe { destroy_imported_ahb_vk_image(raw_device, imported) };

    log::info!(
        "[DIAG-NATIVE-PRESENTER] Vulkan AHB GPU clear for SurfaceControl present ok label={} size={}x{} stride={} vk_format={:?} format_features={:?} allocation_size={} memory_type_bits=0x{:x} memory_type_index={} exported_sync_fd={} queue_family={}",
        label,
        slot.width,
        slot.height,
        slot.stride,
        vk_format,
        format_features,
        allocation_size,
        memory_type_bits,
        memory_type_index,
        exported_sync_fd,
        vk_device.queue_family_index()
    );
    Ok(exported_sync_fd)
}

pub(super) struct ImportedAhbVkImage {
    pub(super) image: ash::vk::Image,
    pub(super) memory: ash::vk::DeviceMemory,
    pub(super) format: ash::vk::Format,
    pub(super) format_features: ash::vk::FormatFeatureFlags,
    pub(super) allocation_size: ash::vk::DeviceSize,
    pub(super) memory_type_bits: u32,
    pub(super) memory_type_index: u32,
}

pub(super) unsafe fn import_ahb_slot_to_vulkan_image(
    vk_device: &wgpu::hal::vulkan::Device,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<ImportedAhbVkImage> {
    let raw_ahb = slot.buffer.as_ptr() as *mut ash::vk::AHardwareBuffer;
    let raw_device = vk_device.raw_device();
    let raw_instance = vk_device.shared_instance().raw_instance();
    let ahb_ext = ash::android::external_memory_android_hardware_buffer::Device::new(
        raw_instance,
        raw_device,
    );

    let (allocation_size, memory_type_bits, vk_format, format_features, external_format) = {
        let mut format_props = ash::vk::AndroidHardwareBufferFormatPropertiesANDROID::default();
        let mut ahb_props =
            ash::vk::AndroidHardwareBufferPropertiesANDROID::default().push_next(&mut format_props);
        unsafe {
            ahb_ext
                .get_android_hardware_buffer_properties(raw_ahb as *const _, &mut ahb_props)
                .map_err(|err| {
                    anyhow::anyhow!(
                        "vkGetAndroidHardwareBufferPropertiesANDROID failed: {:?}",
                        err
                    )
                })?;
        }
        (
            ahb_props.allocation_size,
            ahb_props.memory_type_bits,
            format_props.format,
            format_props.format_features,
            format_props.external_format,
        )
    };

    if allocation_size == 0 || memory_type_bits == 0 {
        return Err(anyhow::anyhow!(
            "AHB Vulkan properties invalid allocation_size={} memory_type_bits=0x{:x}",
            allocation_size,
            memory_type_bits
        ));
    }
    if vk_format == ash::vk::Format::UNDEFINED {
        return Err(anyhow::anyhow!(
            "AHB reported external-only format external_format={} for RGBA8 probe; first import probe only handles concrete Vulkan formats",
            external_format
        ));
    }
    let minimum_format_features =
        ash::vk::FormatFeatureFlags::SAMPLED_IMAGE | ash::vk::FormatFeatureFlags::COLOR_ATTACHMENT;
    if !format_features.contains(minimum_format_features) {
        return Err(anyhow::anyhow!(
            "AHB format {:?} lacks minimum sampled/color-attachment features: have={:?} need={:?}",
            vk_format,
            format_features,
            minimum_format_features
        ));
    }

    let memory_props = unsafe {
        raw_instance.get_physical_device_memory_properties(vk_device.raw_physical_device())
    };
    let memory_type_index = choose_vulkan_memory_type(
        memory_type_bits,
        &memory_props,
        ash::vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "no compatible Vulkan memory type for AHB memory_type_bits=0x{:x}",
            memory_type_bits
        )
    })?;

    let image = create_importable_ahb_vk_image(raw_device, slot.width, slot.height, vk_format)?;
    let memory = match import_ahb_memory_and_bind_vk_image(
        raw_device,
        image,
        raw_ahb,
        allocation_size,
        memory_type_index,
    ) {
        Ok(memory) => memory,
        Err(err) => {
            unsafe { raw_device.destroy_image(image, None) };
            return Err(err);
        }
    };

    Ok(ImportedAhbVkImage {
        image,
        memory,
        format: vk_format,
        format_features,
        allocation_size,
        memory_type_bits,
        memory_type_index,
    })
}

unsafe fn destroy_imported_ahb_vk_image(raw_device: &ash::Device, imported: ImportedAhbVkImage) {
    unsafe {
        raw_device.destroy_image(imported.image, None);
        raw_device.free_memory(imported.memory, None);
    }
}

unsafe fn clear_imported_ahb_vk_image_and_export_sync_fd(
    vk_device: &wgpu::hal::vulkan::Device,
    image: ash::vk::Image,
) -> anyhow::Result<i32> {
    let raw_device = vk_device.raw_device();
    let queue_family = vk_device.queue_family_index();
    let command_pool_info = ash::vk::CommandPoolCreateInfo::default()
        .flags(ash::vk::CommandPoolCreateFlags::TRANSIENT)
        .queue_family_index(queue_family);
    let command_pool = unsafe { raw_device.create_command_pool(&command_pool_info, None) }
        .map_err(|err| anyhow::anyhow!("vkCreateCommandPool(AHB clear probe) failed: {:?}", err))?;

    let result = unsafe {
        clear_imported_ahb_vk_image_and_export_sync_fd_with_pool(vk_device, image, command_pool)
    };
    unsafe { raw_device.destroy_command_pool(command_pool, None) };
    result
}

unsafe fn clear_imported_ahb_vk_image_and_export_sync_fd_with_pool(
    vk_device: &wgpu::hal::vulkan::Device,
    image: ash::vk::Image,
    command_pool: ash::vk::CommandPool,
) -> anyhow::Result<i32> {
    let raw_device = vk_device.raw_device();
    let command_buffer_info = ash::vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(ash::vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let command_buffers = unsafe { raw_device.allocate_command_buffers(&command_buffer_info) }
        .map_err(|err| {
            anyhow::anyhow!(
                "vkAllocateCommandBuffers(AHB clear probe) failed: {:?}",
                err
            )
        })?;
    let command_buffer = command_buffers
        .first()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("vkAllocateCommandBuffers returned no command buffers"))?;

    let mut export_info = ash::vk::ExportSemaphoreCreateInfo::default()
        .handle_types(ash::vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let semaphore_info = ash::vk::SemaphoreCreateInfo::default().push_next(&mut export_info);
    let signal_semaphore =
        unsafe { raw_device.create_semaphore(&semaphore_info, None) }.map_err(|err| {
            anyhow::anyhow!(
                "vkCreateSemaphore(sync-fd export AHB clear probe) failed: {:?}",
                err
            )
        })?;

    let fence_info = ash::vk::FenceCreateInfo::default();
    let fence = match unsafe { raw_device.create_fence(&fence_info, None) } {
        Ok(fence) => fence,
        Err(err) => {
            unsafe { raw_device.destroy_semaphore(signal_semaphore, None) };
            return Err(anyhow::anyhow!(
                "vkCreateFence(AHB clear probe) failed: {:?}",
                err
            ));
        }
    };

    let record_result =
        unsafe { record_ahb_clear_command_buffer(vk_device, command_buffer, image) };
    if let Err(err) = record_result {
        unsafe {
            raw_device.destroy_fence(fence, None);
            raw_device.destroy_semaphore(signal_semaphore, None);
        }
        return Err(err);
    }

    let submit_command_buffers = [command_buffer];
    let submit_signal_semaphores = [signal_semaphore];
    let submit_info = ash::vk::SubmitInfo::default()
        .command_buffers(&submit_command_buffers)
        .signal_semaphores(&submit_signal_semaphores);
    if let Err(err) =
        unsafe { raw_device.queue_submit(vk_device.raw_queue(), &[submit_info], fence) }
    {
        unsafe {
            raw_device.destroy_fence(fence, None);
            raw_device.destroy_semaphore(signal_semaphore, None);
        }
        return Err(anyhow::anyhow!(
            "vkQueueSubmit(AHB clear probe) failed: {:?}",
            err
        ));
    }

    let semaphore_fd_ext = ash::khr::external_semaphore_fd::Device::new(
        vk_device.shared_instance().raw_instance(),
        raw_device,
    );
    let fd_info = ash::vk::SemaphoreGetFdInfoKHR::default()
        .semaphore(signal_semaphore)
        .handle_type(ash::vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let exported_sync_fd = match unsafe { semaphore_fd_ext.get_semaphore_fd(&fd_info) } {
        Ok(fd) => fd,
        Err(err) => {
            let _ = unsafe { raw_device.wait_for_fences(&[fence], true, 5_000_000_000) };
            unsafe {
                raw_device.destroy_fence(fence, None);
                raw_device.destroy_semaphore(signal_semaphore, None);
            }
            return Err(anyhow::anyhow!(
                "vkGetSemaphoreFdKHR(SYNC_FD AHB clear probe) failed: {:?}",
                err
            ));
        }
    };

    // We export the fd before waiting to prove the real acquire-fence primitive.
    // For this diagnostic-only probe we then wait before freeing Vulkan objects;
    // the fd is closed by the caller because it is not yet handed to
    // SurfaceFlinger.
    if let Err(err) = unsafe { raw_device.wait_for_fences(&[fence], true, 5_000_000_000) } {
        if exported_sync_fd >= 0 {
            close_fd(exported_sync_fd);
        }
        unsafe {
            raw_device.destroy_fence(fence, None);
            raw_device.destroy_semaphore(signal_semaphore, None);
        }
        return Err(anyhow::anyhow!(
            "vkWaitForFences(AHB clear probe) failed after exporting sync fd: {:?}",
            err
        ));
    }

    unsafe {
        raw_device.destroy_fence(fence, None);
        raw_device.destroy_semaphore(signal_semaphore, None);
    }
    Ok(exported_sync_fd)
}

unsafe fn record_ahb_clear_command_buffer(
    vk_device: &wgpu::hal::vulkan::Device,
    command_buffer: ash::vk::CommandBuffer,
    image: ash::vk::Image,
) -> anyhow::Result<()> {
    let raw_device = vk_device.raw_device();
    let begin_info = ash::vk::CommandBufferBeginInfo::default()
        .flags(ash::vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    unsafe { raw_device.begin_command_buffer(command_buffer, &begin_info) }.map_err(|err| {
        anyhow::anyhow!("vkBeginCommandBuffer(AHB clear probe) failed: {:?}", err)
    })?;

    let subresource_range = ash::vk::ImageSubresourceRange::default()
        .aspect_mask(ash::vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1);
    let queue_family = vk_device.queue_family_index();
    let acquire_barrier = ash::vk::ImageMemoryBarrier::default()
        .src_access_mask(ash::vk::AccessFlags::empty())
        .dst_access_mask(ash::vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(ash::vk::ImageLayout::UNDEFINED)
        .new_layout(ash::vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_queue_family_index(ash::vk::QUEUE_FAMILY_FOREIGN_EXT)
        .dst_queue_family_index(queue_family)
        .image(image)
        .subresource_range(subresource_range);
    unsafe {
        raw_device.cmd_pipeline_barrier(
            command_buffer,
            ash::vk::PipelineStageFlags::TOP_OF_PIPE,
            ash::vk::PipelineStageFlags::TRANSFER,
            ash::vk::DependencyFlags::empty(),
            &[],
            &[],
            &[acquire_barrier],
        );
    }

    let clear_color = ash::vk::ClearColorValue {
        float32: [0.02, 0.20, 0.95, 1.0],
    };
    unsafe {
        raw_device.cmd_clear_color_image(
            command_buffer,
            image,
            ash::vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &clear_color,
            &[subresource_range],
        );
    }

    let release_barrier = ash::vk::ImageMemoryBarrier::default()
        .src_access_mask(ash::vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(ash::vk::AccessFlags::empty())
        .old_layout(ash::vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(ash::vk::ImageLayout::GENERAL)
        .src_queue_family_index(queue_family)
        .dst_queue_family_index(ash::vk::QUEUE_FAMILY_FOREIGN_EXT)
        .image(image)
        .subresource_range(subresource_range);
    unsafe {
        raw_device.cmd_pipeline_barrier(
            command_buffer,
            ash::vk::PipelineStageFlags::TRANSFER,
            ash::vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            ash::vk::DependencyFlags::empty(),
            &[],
            &[],
            &[release_barrier],
        );
    }

    unsafe { raw_device.end_command_buffer(command_buffer) }
        .map_err(|err| anyhow::anyhow!("vkEndCommandBuffer(AHB clear probe) failed: {:?}", err))
}

fn create_importable_ahb_vk_image(
    raw_device: &ash::Device,
    width: u32,
    height: u32,
    format: ash::vk::Format,
) -> anyhow::Result<ash::vk::Image> {
    let mut external_memory = ash::vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(ash::vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);
    let image_info = ash::vk::ImageCreateInfo::default()
        .push_next(&mut external_memory)
        .image_type(ash::vk::ImageType::TYPE_2D)
        .format(format)
        .extent(ash::vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(ash::vk::SampleCountFlags::TYPE_1)
        .tiling(ash::vk::ImageTiling::OPTIMAL)
        .usage(
            ash::vk::ImageUsageFlags::TRANSFER_DST
                | ash::vk::ImageUsageFlags::COLOR_ATTACHMENT
                | ash::vk::ImageUsageFlags::SAMPLED,
        )
        .sharing_mode(ash::vk::SharingMode::EXCLUSIVE)
        .initial_layout(ash::vk::ImageLayout::UNDEFINED);

    unsafe { raw_device.create_image(&image_info, None) }
        .map_err(|err| anyhow::anyhow!("vkCreateImage(AHB importable) failed: {:?}", err))
}

fn import_ahb_memory_and_bind_vk_image(
    raw_device: &ash::Device,
    image: ash::vk::Image,
    raw_ahb: *mut ash::vk::AHardwareBuffer,
    allocation_size: ash::vk::DeviceSize,
    memory_type_index: u32,
) -> anyhow::Result<ash::vk::DeviceMemory> {
    let mut import_ahb = ash::vk::ImportAndroidHardwareBufferInfoANDROID::default().buffer(raw_ahb);
    let mut dedicated = ash::vk::MemoryDedicatedAllocateInfo::default().image(image);
    let allocate_info = ash::vk::MemoryAllocateInfo::default()
        .push_next(&mut import_ahb)
        .push_next(&mut dedicated)
        .allocation_size(allocation_size)
        .memory_type_index(memory_type_index);
    let memory = unsafe { raw_device.allocate_memory(&allocate_info, None) }
        .map_err(|err| anyhow::anyhow!("vkAllocateMemory(AHB import) failed: {:?}", err))?;
    if let Err(err) = unsafe { raw_device.bind_image_memory(image, memory, 0) } {
        unsafe { raw_device.free_memory(memory, None) };
        return Err(anyhow::anyhow!(
            "vkBindImageMemory(AHB import) failed: {:?}",
            err
        ));
    }
    Ok(memory)
}

fn choose_vulkan_memory_type(
    memory_type_bits: u32,
    memory_props: &ash::vk::PhysicalDeviceMemoryProperties,
    preferred_flags: ash::vk::MemoryPropertyFlags,
) -> Option<u32> {
    let mut first_compatible = None;
    for index in 0..memory_props.memory_type_count {
        if (memory_type_bits & (1u32 << index)) == 0 {
            continue;
        }
        if first_compatible.is_none() {
            first_compatible = Some(index);
        }
        let flags = memory_props.memory_types[index as usize].property_flags;
        if flags.contains(preferred_flags) {
            return Some(index);
        }
    }
    first_compatible
}

pub(crate) fn probe_wgpu_custom_vulkan_device_extensions(adapter: &wgpu::Adapter) {
    let features = default_native_presenter_wgpu_features(adapter);
    let (device, queue) = match create_wgpu_custom_vulkan_device_with_android_interop(
        adapter,
        features,
        "Dyxel Android native presenter custom Vulkan device probe",
    ) {
        Ok(created) => created,
        Err(err) => {
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] custom Vulkan device probe failed: {:?}",
                err
            );
            return;
        }
    };

    log_wgpu_vulkan_external_ahb_support(adapter, &device);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    drop(queue);
    drop(device);
    log::info!(
        "[DIAG-NATIVE-PRESENTER] custom Vulkan device probe succeeded and dropped test device"
    );
}

pub(crate) fn default_native_presenter_wgpu_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    adapter.features() & (wgpu::Features::CLEAR_TEXTURE | wgpu::Features::PIPELINE_CACHE)
}

pub(crate) fn create_wgpu_custom_vulkan_device_with_android_interop(
    adapter: &wgpu::Adapter,
    features: wgpu::Features,
    label: &'static str,
) -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    let memory_hints = wgpu::MemoryHints::default();
    let open_device = {
        let Some(vk_adapter) = (unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }) else {
            return Err(anyhow::anyhow!("adapter is not Vulkan"));
        };
        let caps = vk_adapter.physical_device_capabilities();
        let api_version = caps.properties().api_version;
        let available =
            VulkanNativePresenterExtensionSupport::from_support_predicate(api_version, |name| {
                caps.supports_extension(vk_ext_name(name))
            });
        let missing = available.required_missing();
        if !missing.is_empty() {
            return Err(anyhow::anyhow!(
                "physical device missing {}",
                missing.join(", ")
            ));
        }

        let extensions_to_request =
            custom_device_explicit_extensions(|name| caps.supports_extension(name));
        let extension_names = extensions_to_request
            .iter()
            .filter_map(|ext| ext.to_str().ok())
            .collect::<Vec<_>>()
            .join(", ");
        log::info!(
            "[DIAG-NATIVE-PRESENTER] custom Vulkan device requesting explicit extensions: {}",
            extension_names
        );

        let callback_extensions = extensions_to_request.clone();
        unsafe {
            vk_adapter.open_with_callback(
                features,
                &memory_hints,
                Some(Box::new(move |args| {
                    for extension in callback_extensions {
                        if !args.extensions.iter().any(|enabled| *enabled == extension) {
                            args.extensions.push(extension);
                        }
                    }
                })),
            )
        }
        .map_err(|err| anyhow::anyhow!("open_with_callback failed: {:?}", err))?
    };

    let desc = wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: features,
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    };
    unsafe { adapter.create_device_from_hal::<wgpu::hal::api::Vulkan>(open_device, &desc) }
        .map_err(|err| anyhow::anyhow!("create_device_from_hal failed: {:?}", err))
}

fn custom_device_explicit_extensions(
    mut supports: impl FnMut(&'static CStr) -> bool,
) -> Vec<&'static CStr> {
    [
        VK_EXT_AHB,
        VK_EXT_EXTERNAL_MEMORY,
        VK_EXT_EXTERNAL_MEMORY_FD,
        VK_EXT_EXTERNAL_SEMAPHORE,
        VK_EXT_EXTERNAL_SEMAPHORE_FD,
        VK_EXT_DEDICATED_ALLOCATION,
        VK_EXT_GET_MEMORY_REQUIREMENTS2,
        VK_EXT_BIND_MEMORY2,
        VK_EXT_QUEUE_FAMILY_FOREIGN,
        VK_EXT_SAMPLER_YCBCR_CONVERSION,
    ]
    .into_iter()
    .map(vk_ext_name)
    .filter(|name| supports(name))
    .collect()
}
