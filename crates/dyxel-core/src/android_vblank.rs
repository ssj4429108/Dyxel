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
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex as StdMutex;
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
    scheduler_event_tx: StdMutex<Option<crossbeam_channel::Sender<crate::frame_scheduler::SchedulerEvent>>>,
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
            scheduler_event_tx: StdMutex::new(None),
        });
        *ANDROID_VBLANK.lock().unwrap() = Some(waiter.clone());
        waiter
    }

    /// Set the scheduler event sender so VBlank signals also drive the FrameScheduler.
    pub fn set_scheduler_tx(&self, tx: crossbeam_channel::Sender<crate::frame_scheduler::SchedulerEvent>) {
        *self.scheduler_event_tx.lock().unwrap() = Some(tx);
    }

    /// Called from the Kotlin UI thread (Choreographer.doFrame) via JNI.
    /// `refresh_hz` is the display's current refresh rate (e.g. from
    /// `Display.getRefreshRate()`).
    /// This must be lock-free and non-blocking on the UI thread.
    pub fn on_vblank(&self, refresh_hz: f64) {
        self.state.counter.fetch_add(1, Ordering::Release);
        let _guard = self.state.mutex.lock().unwrap();
        self.state.condvar.notify_all();

        // Also notify the FrameScheduler so cadence control is VBlank-driven.
        if let Ok(lock) = self.scheduler_event_tx.lock() {
            if let Some(ref tx) = *lock {
                let _ = tx.send(crate::frame_scheduler::SchedulerEvent::VBlank {
                    timestamp: std::time::Instant::now(),
                    refresh_hz,
                });
            }
        }
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

/// Set the scheduler event sender on the globally registered AndroidVBlankWaiter.
/// Called from bridge.rs after the FrameScheduler is created.
pub fn set_scheduler_tx(tx: crossbeam_channel::Sender<crate::frame_scheduler::SchedulerEvent>) {
    if let Some(waiter) = ANDROID_VBLANK.lock().unwrap().as_ref() {
        waiter.set_scheduler_tx(tx);
    }
}

/// JNI entry point called by `Choreographer.FrameCallback` on the Android UI thread.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_com_dyxel_android_DyxelEngine_nativeOnVBlank(
    _env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jobject,
    refresh_hz: f64,
) {
    if let Some(waiter) = ANDROID_VBLANK.lock().unwrap().as_ref() {
        waiter.on_vblank(refresh_hz);
    }
}
