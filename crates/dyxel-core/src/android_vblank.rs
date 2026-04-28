// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Android Choreographer-based VBlank waiter.
//!
//! Implements the same Atomic Counter Fence pattern as `MacVBlankWaiter`:
//! the Kotlin UI thread calls `nativeOnVBlank()` from `Choreographer.doFrame`,
//! which atomically increments a counter and notifies the render-thread VBlank
//! forwarder.
//!
//! This keeps Android's RenderThread perfectly aligned with the display's VSync
//! without relying on coarse `thread::sleep` timers.

use crate::pacer::VBlankWaiter;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex as StdMutex;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Shared physical VBlank state.
struct VBlankState {
    /// Monotonically increasing VBlank counter.
    counter: AtomicU64,
    /// Used to park the render thread.
    condvar: Condvar,
    /// Mutex paired with `condvar`.
    mutex: Mutex<u64>,
}

/// Android Choreographer-based VBlank waiter.
pub struct AndroidVBlankWaiter {
    state: Arc<VBlankState>,
    scheduler_event_tx:
        StdMutex<Option<crossbeam_channel::Sender<crate::frame_scheduler::SchedulerEvent>>>,
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
                mutex: Mutex::new(0),
            }),
            scheduler_event_tx: StdMutex::new(None),
        });
        *ANDROID_VBLANK.lock().unwrap() = Some(waiter.clone());
        waiter
    }

    /// Deprecated compatibility hook. The scheduler is driven by the render
    /// thread's VBlank forwarding loop so each physical VBlank produces one
    /// scheduler event.
    pub fn set_scheduler_tx(
        &self,
        tx: crossbeam_channel::Sender<crate::frame_scheduler::SchedulerEvent>,
    ) {
        *self.scheduler_event_tx.lock().unwrap() = Some(tx);
    }

    /// Called from the Kotlin UI thread (Choreographer.doFrame) via JNI.
    /// `refresh_hz` is the display's current refresh rate (e.g. from
    /// `Display.getRefreshRate()`).
    /// This must be lock-free and non-blocking on the UI thread.
    pub fn on_vblank(&self, _refresh_hz: f64) {
        let new_counter = self.state.counter.fetch_add(1, Ordering::Release) + 1;
        if let Ok(mut guard) = self.state.mutex.lock() {
            *guard = new_counter;
            self.state.condvar.notify_all();
        }

        // Do not send SchedulerEvent::VBlank here. The render-thread
        // forwarding loop waits on this counter and is the single source of
        // scheduler VBlank events.
    }
}

impl VBlankWaiter for AndroidVBlankWaiter {
    fn wait_for_vblank(&self) {
        // Capture the counter at the moment we start waiting.
        // We want to wait for a VBlank that fires *after* this call,
        // not "catch up" to stale VBlanks that arrived while we were
        // busy rendering the previous frame.
        let start_counter = self.state.counter.load(Ordering::Acquire);
        let target = start_counter + 1;

        // Slow path: parked wait with timeout to prevent lost-wakeup deadlock.
        // Use 100ms timeout (same as MacVBlankWaiter) — much longer than any
        // reasonable frame interval, so normal operation never times out.
        let timeout = Duration::from_millis(100);
        let guard = self.state.mutex.lock().unwrap();

        // If we're already at or past target, no need to wait.
        // This handles the case where a VBlank fired between our
        // load of start_counter and our acquisition of the mutex.
        if *guard >= target {
            return;
        }

        let result = self
            .state
            .condvar
            .wait_timeout_while(guard, timeout, |c| *c < target);

        match result {
            Ok((new_guard, timeout_result)) => {
                if timeout_result.timed_out() {
                    log::warn!(
                        "AndroidVBlankWaiter: condvar timed out after {:?} (counter={} target={}). \
                         Choreographer may be stalled or display is off.",
                        timeout,
                        self.state.counter.load(Ordering::Acquire),
                        target
                    );
                }
                // Drop the guard (not used beyond this point)
                drop(new_guard);
            }
            Err(poisoned) => {
                log::error!("AndroidVBlankWaiter: mutex poisoned");
                let _ = poisoned.into_inner();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_vblank_does_not_emit_scheduler_event_directly() {
        let _ = AndroidVBlankWaiter::new();
        let waiter = ANDROID_VBLANK
            .lock()
            .unwrap()
            .as_ref()
            .expect("waiter should be registered")
            .clone();
        let (tx, rx) = crossbeam_channel::unbounded();

        waiter.set_scheduler_tx(tx);
        waiter.on_vblank(60.0);

        assert!(
            rx.try_recv().is_err(),
            "on_vblank should only wake the waiter; the forwarding thread is the single scheduler VBlank source"
        );
    }
}
