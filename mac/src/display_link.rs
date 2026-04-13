// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_core::pacer::VBlankWaiter;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

// CoreVideo FFI types
#[allow(non_camel_case_types)]
type CVDisplayLinkRef = *mut c_void;
#[allow(non_camel_case_types)]
type CVReturn = i32;
#[allow(non_camel_case_types)]
type CVOptionFlags = u64;

const kCVReturnSuccess: CVReturn = 0;

#[repr(C)]
#[allow(dead_code)]
struct CVTimeStamp {
    version: u32,
    video_time_scale: i32,
    video_time: i64,
    host_time: u64,
    rate_scalar: i64,
    video_refresh_period: i64,
    smpte_time: CVSMPTETime,
    flags: u64,
    reserved: u64,
}

#[repr(C)]
#[allow(dead_code)]
struct CVSMPTETime {
    subframes: i16,
    subframe_divisor: i16,
    counter: u32,
    _type: u32,
    flags: u32,
    hours: i16,
    minutes: i16,
    seconds: i16,
    frames: i16,
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVDisplayLinkCreateWithActiveCGDisplays(link: *mut CVDisplayLinkRef) -> CVReturn;
    fn CVDisplayLinkSetOutputCallback(
        link: CVDisplayLinkRef,
        callback: extern "C" fn(
            CVDisplayLinkRef,
            *const CVTimeStamp,
            *const CVTimeStamp,
            CVOptionFlags,
            *mut CVOptionFlags,
            *mut c_void,
        ) -> CVReturn,
        context: *mut c_void,
    ) -> CVReturn;
    fn CVDisplayLinkStart(link: CVDisplayLinkRef) -> CVReturn;
    fn CVDisplayLinkStop(link: CVDisplayLinkRef) -> CVReturn;
    fn CVDisplayLinkRelease(link: CVDisplayLinkRef);
}

extern "C" fn display_link_callback(
    _link: CVDisplayLinkRef,
    _in_now: *const CVTimeStamp,
    _in_output_time: *const CVTimeStamp,
    _flags_in: CVOptionFlags,
    _flags_out: *mut CVOptionFlags,
    context: *mut c_void,
) -> CVReturn {
    let state = unsafe { &*(context as *const VBlankState) };
    state.counter.fetch_add(1, Ordering::SeqCst);
    let _ = state.condvar.notify_one();
    kCVReturnSuccess
}

struct VBlankState {
    counter: AtomicU64,
    condvar: Condvar,
}

/// macOS CVDisplayLink-based VBlank waiter.
pub struct MacVBlankWaiter {
    display_link: CVDisplayLinkRef,
    state: Arc<VBlankState>,
    last_counter: Mutex<u64>,
}

unsafe impl Send for MacVBlankWaiter {}
unsafe impl Sync for MacVBlankWaiter {}

impl MacVBlankWaiter {
    pub fn new() -> anyhow::Result<Arc<dyn VBlankWaiter>> {
        let mut display_link: CVDisplayLinkRef = std::ptr::null_mut();
        let result = unsafe { CVDisplayLinkCreateWithActiveCGDisplays(&mut display_link) };
        if result != kCVReturnSuccess || display_link.is_null() {
            return Err(anyhow::anyhow!(
                "CVDisplayLinkCreateWithActiveCGDisplays failed: {}",
                result
            ));
        }

        let state = Arc::new(VBlankState {
            counter: AtomicU64::new(0),
            condvar: Condvar::new(),
        });
        let state_ptr = Arc::into_raw(state.clone()) as *mut c_void;

        let cb_result = unsafe {
            CVDisplayLinkSetOutputCallback(display_link, display_link_callback, state_ptr)
        };
        if cb_result != kCVReturnSuccess {
            unsafe {
                CVDisplayLinkRelease(display_link);
                let _ = Arc::from_raw(state_ptr as *const VBlankState);
            }
            return Err(anyhow::anyhow!(
                "CVDisplayLinkSetOutputCallback failed: {}",
                cb_result
            ));
        }

        let start_result = unsafe { CVDisplayLinkStart(display_link) };
        if start_result != kCVReturnSuccess {
            unsafe {
                CVDisplayLinkRelease(display_link);
                let _ = Arc::from_raw(state_ptr as *const VBlankState);
            }
            return Err(anyhow::anyhow!(
                "CVDisplayLinkStart failed: {}",
                start_result
            ));
        }

        Ok(Arc::new(MacVBlankWaiter {
            display_link,
            state,
            last_counter: Mutex::new(0),
        }))
    }
}

impl VBlankWaiter for MacVBlankWaiter {
    fn wait_for_vblank(&self) {
        let mut last = self.last_counter.lock().unwrap();
        // Wait for a VBlank that fires *after* this call starts.
        // This prevents us from "catching up" to stale VBlanks that fired
        // while the render thread was busy with the previous frame.
        let start_counter = self.state.counter.load(Ordering::SeqCst);
        let target = start_counter + 1;
        last = self
            .state
            .condvar
            .wait_while(last, |l| self.state.counter.load(Ordering::SeqCst) < target)
            .unwrap();
        *last = target;
    }
}

impl Drop for MacVBlankWaiter {
    fn drop(&mut self) {
        unsafe {
            CVDisplayLinkStop(self.display_link);
            CVDisplayLinkRelease(self.display_link);
        }
    }
}
