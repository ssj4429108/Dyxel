// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounded presenter for Android full-frame offscreen rendering.
//!
//! The render thread produces app-owned offscreen frames. This presenter owns
//! the Android surface acquire/blit/present path, waits for GPU readiness, and
//! keeps the queue bounded so producer work cannot run indefinitely ahead of
//! SurfaceFlinger/HWC consumption.

use crate::frame_scheduler::SchedulerEvent;
use crate::renderer::{DeferredRenderFrame, RenderFrameTimings};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[cfg(target_os = "android")]
pub fn android_full_frame_offscreen_enabled() -> bool {
    std::env::var("DYXEL_ANDROID_FULL_FRAME_OFFSCREEN")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "android"))]
pub fn android_full_frame_offscreen_enabled() -> bool {
    false
}

#[cfg(target_os = "android")]
pub fn android_detached_present_enabled() -> bool {
    std::env::var("DYXEL_ANDROID_DETACHED_PRESENT")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "android"))]
pub fn android_detached_present_enabled() -> bool {
    false
}

const MAX_QUEUED_FRAMES: usize = 2;
const GPU_READY_WAIT_TIMEOUT: Duration = Duration::from_millis(50);

pub struct QueuedPresentFrame {
    pub frame_id: u64,
    pub epoch: u64,
    pub frame: DeferredRenderFrame,
}

#[derive(Clone)]
pub struct BoundedPresenter {
    inner: Arc<PresenterInner>,
}

struct PresenterInner {
    queue: Mutex<VecDeque<QueuedPresentFrame>>,
    cv: Condvar,
    shutdown: AtomicBool,
}

impl BoundedPresenter {
    pub fn spawn(
        scheduler_tx: crossbeam_channel::Sender<SchedulerEvent>,
        frame_perf_state: Arc<Mutex<dyxel_perf::FramePerformanceStats>>,
    ) -> Self {
        let inner = Arc::new(PresenterInner {
            queue: Mutex::new(VecDeque::with_capacity(MAX_QUEUED_FRAMES)),
            cv: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });
        let thread_inner = inner.clone();
        std::thread::Builder::new()
            .name("DyxelPresenter".into())
            .spawn(move || presenter_loop(thread_inner, scheduler_tx, frame_perf_state))
            .expect("Failed to spawn presenter thread");
        Self { inner }
    }

    pub fn wait_for_capacity(&self) -> bool {
        let mut queue = self.inner.queue.lock().unwrap();
        while queue.len() >= MAX_QUEUED_FRAMES && !self.inner.shutdown.load(Ordering::Acquire) {
            queue = self.inner.cv.wait(queue).unwrap();
        }
        !self.inner.shutdown.load(Ordering::Acquire)
    }

    pub fn submit(&self, frame: QueuedPresentFrame) {
        let mut queue = self.inner.queue.lock().unwrap();
        if queue.len() >= MAX_QUEUED_FRAMES {
            // The scheduler should prevent this. If it happens, drop the oldest
            // frame and release its pipeline slot to preserve bounded memory.
            if let Some(dropped) = queue.pop_front() {
                log::warn!(
                    "BoundedPresenter: dropping over-capacity frame_id={}",
                    dropped.frame_id
                );
            }
        }
        queue.push_back(frame);
        self.inner.cv.notify_all();
    }

    pub fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::Release);
        self.inner.cv.notify_all();
    }
}

fn presenter_loop(
    inner: Arc<PresenterInner>,
    scheduler_tx: crossbeam_channel::Sender<SchedulerEvent>,
    frame_perf_state: Arc<Mutex<dyxel_perf::FramePerformanceStats>>,
) {
    let mut raster_frame_buffer = dyxel_perf::EventRateBuffer::new(60);
    loop {
        let frame = {
            let mut queue = inner.queue.lock().unwrap();
            while queue.is_empty() && !inner.shutdown.load(Ordering::Acquire) {
                queue = inner.cv.wait(queue).unwrap();
            }
            if inner.shutdown.load(Ordering::Acquire) {
                return;
            }
            let frame = queue.pop_front();
            inner.cv.notify_all();
            frame
        };

        let Some(frame) = frame else {
            continue;
        };
        present_frame(
            &scheduler_tx,
            &frame_perf_state,
            &mut raster_frame_buffer,
            frame,
        );
    }
}

fn present_frame(
    scheduler_tx: &crossbeam_channel::Sender<SchedulerEvent>,
    frame_perf_state: &Arc<Mutex<dyxel_perf::FramePerformanceStats>>,
    raster_frame_buffer: &mut dyxel_perf::EventRateBuffer,
    frame: QueuedPresentFrame,
) {
    let frame_id = frame.frame_id;
    let epoch = frame.epoch;

    let (gpu_ready, gpu_ready_wait_ms) = frame.frame.wait_until_gpu_ready(GPU_READY_WAIT_TIMEOUT);
    if !gpu_ready || gpu_ready_wait_ms >= 8.0 || frame_id % 20 == 0 {
        log::info!(
            "[DIAG] Presenter gpu_ready frame_id={} ready={} wait={:.2}ms",
            frame_id,
            gpu_ready,
            gpu_ready_wait_ms
        );
    }
    let _ = scheduler_tx.send(SchedulerEvent::GpuReady { frame_id, epoch });

    let timings = frame.frame.present();
    let _ = scheduler_tx.send(SchedulerEvent::Presented {
        frame_id,
        epoch,
        present_ms: timings.end_ms as f32,
    });

    raster_frame_buffer.push(Instant::now());
    if let Ok(mut perf) = frame_perf_state.lock() {
        perf.raster_fps = raster_frame_buffer.fps();
    }

    if timings.end_ms >= 8.0 || frame_id % 20 == 0 {
        log::info!(
            "[DIAG] Presenter frame_id={} present={:.2}ms backend={:.2}ms",
            frame_id,
            timings.end_ms,
            timings.backend_ms
        );
    }
}

pub fn present_synchronously(
    frame_id: u64,
    epoch: u64,
    frame: DeferredRenderFrame,
    scheduler_tx: &crossbeam_channel::Sender<SchedulerEvent>,
    raster_frame_buffer: &mut dyxel_perf::EventRateBuffer,
    frame_perf_state: &Arc<Mutex<dyxel_perf::FramePerformanceStats>>,
) -> RenderFrameTimings {
    let _ = scheduler_tx.send(SchedulerEvent::GpuReady { frame_id, epoch });
    let timings = frame.present();
    let _ = scheduler_tx.send(SchedulerEvent::Presented {
        frame_id,
        epoch,
        present_ms: timings.end_ms as f32,
    });
    raster_frame_buffer.push(Instant::now());
    if let Ok(mut perf) = frame_perf_state.lock() {
        perf.raster_fps = raster_frame_buffer.fps();
    }
    timings
}
