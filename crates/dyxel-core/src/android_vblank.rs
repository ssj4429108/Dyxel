// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Android Choreographer-based VBlank waiter.
//!
//! Implements the same Atomic Counter Fence pattern as `MacVBlankWaiter`:
//! the Kotlin UI thread calls `nativeOnVBlank()` from `Choreographer.doFrame`,
//! which atomically increments a counter and notifies any waiting render thread.
//!
//! This keeps Android's RenderThread perfectly aligned with the display's VSync
//! without relying on coarse `thread::sleep` timers.

use crate::pacer::VBlankWaiter;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Shared physical VBlank state.
struct VBlankState {
    /// Monotonically increasing VBlank counter.
    counter: AtomicU64,
    /// Used to park the render thread.
    condvar: Condvar,
    /// Mutex paired with `condvar`.
    mutex: Mutex<()>,
}

/// Android Choreographer-based VBlank waiter.
pub struct AndroidVBlankWaiter {
    state: Arc<VBlankState>,
}

unsafe impl Send for AndroidVBlankWaiter {}
unsafe impl Sync for AndroidVBlankWaiter {}

static ANDROID_VBLANK: Mutex<Option<Arc<AndroidVBlankWaiter>>> = Mutex::new(None);

impl AndroidVBlankWaiter {
    /// Create a new waiter, register it globally so the JNI callback can reach it,
    /// and return it as a trait object for the pacer.
    pub fn new() -> Arc<dyn VBlankWaiter> {
        let waiter = Arc::new(AndroidVBlankWaiter {
            state: Arc::new(VBlankState {
                counter: AtomicU64::new(0),
                condvar: Condvar::new(),
                mutex: Mutex::new(()),
            }),
        });
        *ANDROID_VBLANK.lock().unwrap() = Some(waiter.clone());
        waiter
    }

    /// Called from the Kotlin UI thread (Choreographer.doFrame) via JNI.
    /// This must be lock-free and non-blocking on the UI thread.
    pub fn on_vblank(&self) {
        self.state.counter.fetch_add(1, Ordering::Release);
        let _guard = self.state.mutex.lock().unwrap();
        self.state.condvar.notify_all();
    }
}

impl VBlankWaiter for AndroidVBlankWaiter {
    fn wait_for_vblank(&self) {
        let current = self.state.counter.load(Ordering::Acquire);
        let target = current + 1;

        // Fast path: already reached target before blocking
        if self.state.counter.load(Ordering::Acquire) >= target {
            return;
        }

        // Slow path: parked wait with timeout fence to prevent lost-wakeup deadlock.
        // 8 ms is roughly half of a 60 Hz frame (16.67 ms).
        let timeout = Duration::from_millis(8);
        let mut guard = self.state.mutex.lock().unwrap();

        while self.state.counter.load(Ordering::Acquire) < target {
            let (new_guard, wait_result) = self.state.condvar.wait_timeout(guard, timeout).unwrap();
            guard = new_guard;

            if wait_result.timed_out() {
                continue;
            }
        }
    }
}

/// JNI entry point called by `Choreographer.FrameCallback` on the Android UI thread.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_com_dyxel_android_DyxelEngine_nativeOnVBlank(
    _env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jobject,
) {
    if let Some(waiter) = ANDROID_VBLANK.lock().unwrap().as_ref() {
        waiter.on_vblank();
    }
}
