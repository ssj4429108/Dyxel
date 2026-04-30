// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Render-thread frame execution.

use crate::engine::RenderState;
use crate::frame_scheduler::{FrameToken, SchedulerEvent};
use crate::platform::SurfaceId;
use crate::render_mailbox::RenderMailbox;
use dyxel_render_api::{RuntimeSurfaceId, SharedMutex, SharedPtr};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

thread_local! {
    static LAST_RENDER_TOKEN_TIME: std::cell::Cell<Option<Instant>> = std::cell::Cell::new(None);
}

pub struct RenderTokenExecution<'a> {
    pub token: FrameToken,
    pub render_state: &'a mut Option<RenderState>,
    pub active_surface_id: Option<SurfaceId>,
    pub surface_id_map: &'a SharedPtr<SharedMutex<HashMap<u64, RuntimeSurfaceId>>>,
    pub mailbox: &'a Arc<RenderMailbox>,
    pub scheduler_tx: &'a crossbeam_channel::Sender<SchedulerEvent>,
    pub render_complete_tx: &'a mpsc::Sender<()>,
    pub is_rendering: &'a Arc<AtomicBool>,
    pub frame_perf_state: &'a Arc<StdMutex<dyxel_perf::FramePerformanceStats>>,
    pub raster_frame_buffer: &'a mut dyxel_perf::EventRateBuffer,
    pub continuous_render: bool,
    pub presenter: Option<&'a crate::presenter::BoundedPresenter>,
}

pub fn execute_render_token(ctx: RenderTokenExecution<'_>) {
    let RenderTokenExecution {
        token,
        render_state,
        active_surface_id,
        surface_id_map,
        mailbox,
        scheduler_tx,
        render_complete_tx,
        is_rendering,
        frame_perf_state,
        raster_frame_buffer,
        continuous_render,
        presenter,
    } = ctx;

    let token_receive_time = Instant::now();
    let token_latency_ms = token_receive_time
        .duration_since(token.vblank_at)
        .as_secs_f64()
        * 1000.0;
    if token.frame_id % 20 == 0 {
        log::info!(
            "[DIAG] RenderThread token_latency={:.2}ms frame_id={}",
            token_latency_ms,
            token.frame_id
        );
    }

    let frame_interval_ms = LAST_RENDER_TOKEN_TIME.with(|t| {
        let interval = t
            .get()
            .map(|last| token_receive_time.duration_since(last).as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        t.set(Some(token_receive_time));
        interval
    });

    is_rendering.store(true, Ordering::Release);
    let _ = scheduler_tx.send(SchedulerEvent::RenderStarted {
        frame_id: token.frame_id,
        epoch: token.epoch,
    });

    if continuous_render {
        let _ = scheduler_tx.send(SchedulerEvent::InputArrived(0));
    }

    let mut frame_time_ms = 0.0f32;
    let mut did_submit = false;
    let mut did_present_sync = false;
    let render_start = Instant::now();

    if let (Some(render), Some(surface_id)) = (render_state.as_mut(), active_surface_id) {
        let runtime_surface_id = surface_id_map
            .lock()
            .ok()
            .and_then(|map| map.get(&surface_id.0).copied());

        if let Some(runtime_surface_id) = runtime_surface_id {
            let (_mailbox_epoch, package) = mailbox.snapshot();
            let perf_stats = frame_perf_state.lock().ok().map(|perf| *perf);

            #[cfg(target_os = "android")]
            if let Some(presenter) = presenter {
                if presenter.wait_for_capacity() {
                    if let Some(frame) = crate::renderer::render_frame_with_package_deferred_present(
                        render,
                        runtime_surface_id,
                        &package,
                        Some((0.0, frame_interval_ms)),
                        perf_stats,
                    ) {
                        let submitted = frame.submitted_timings();
                        frame_time_ms = submitted.backend_ms as f32;
                        did_submit = true;
                        send_gpu_submitted(
                            scheduler_tx,
                            render_complete_tx,
                            is_rendering,
                            token.frame_id,
                            token.epoch,
                            frame_time_ms,
                            true,
                        );
                        presenter.submit(crate::presenter::QueuedPresentFrame {
                            frame_id: token.frame_id,
                            epoch: token.epoch,
                            frame,
                        });
                    }
                }
            } else {
                did_present_sync = render_sync(
                    render,
                    runtime_surface_id,
                    &package,
                    frame_interval_ms,
                    perf_stats,
                    token.frame_id,
                    token.epoch,
                    scheduler_tx,
                    raster_frame_buffer,
                    frame_perf_state,
                    &mut frame_time_ms,
                );
                did_submit = true;
            }

            #[cfg(not(target_os = "android"))]
            {
                let _ = presenter;
                did_present_sync = render_sync(
                    render,
                    runtime_surface_id,
                    &package,
                    frame_interval_ms,
                    perf_stats,
                    token.frame_id,
                    token.epoch,
                    scheduler_tx,
                    raster_frame_buffer,
                    frame_perf_state,
                    &mut frame_time_ms,
                );
                did_submit = true;
            }
        }
    }

    if !did_submit {
        frame_time_ms = render_start.elapsed().as_secs_f32() * 1000.0;
        send_gpu_submitted(
            scheduler_tx,
            render_complete_tx,
            is_rendering,
            token.frame_id,
            token.epoch,
            frame_time_ms,
            false,
        );
    }

    let total_pipeline_ms = token_receive_time.elapsed().as_secs_f64() * 1000.0;
    let cleanup_ms =
        (render_start.elapsed().as_secs_f64() * 1000.0 - frame_time_ms as f64).max(0.0);
    if token.frame_id % 20 == 0 {
        log::info!(
            "[DIAG] RenderThread pipeline frame_id={} setup=0.00ms render={:.2}ms cleanup={:.2}ms total={:.2}ms",
            token.frame_id,
            frame_time_ms,
            cleanup_ms,
            total_pipeline_ms
        );
    }

    if did_present_sync {
        let _ = render_complete_tx.send(());
    }
    is_rendering.store(false, Ordering::Release);
}

#[allow(clippy::too_many_arguments)]
fn render_sync(
    render: &mut RenderState,
    runtime_surface_id: RuntimeSurfaceId,
    package: &dyxel_render_api::RenderPackage,
    frame_interval_ms: f64,
    perf_stats: Option<dyxel_perf::FramePerformanceStats>,
    frame_id: u64,
    epoch: u64,
    scheduler_tx: &crossbeam_channel::Sender<SchedulerEvent>,
    raster_frame_buffer: &mut dyxel_perf::EventRateBuffer,
    frame_perf_state: &Arc<StdMutex<dyxel_perf::FramePerformanceStats>>,
    frame_time_ms: &mut f32,
) -> bool {
    let render_timings = crate::renderer::render_frame_with_package(
        render,
        runtime_surface_id,
        package,
        Some((0.0, frame_interval_ms)),
        perf_stats,
    );
    *frame_time_ms = render_timings
        .map(|t| {
            #[cfg(any(target_os = "macos", target_os = "android"))]
            {
                t.backend_ms as f32
            }
            #[cfg(not(any(target_os = "macos", target_os = "android")))]
            {
                (t.backend_ms + t.end_ms) as f32
            }
        })
        .unwrap_or(0.0);
    let stats = crate::FrameStats {
        frame_time_ms: *frame_time_ms,
        ..Default::default()
    };
    let _ = scheduler_tx.send(SchedulerEvent::RenderCompleted {
        frame_id,
        epoch,
        stats,
    });
    raster_frame_buffer.push(Instant::now());
    if let Ok(mut perf) = frame_perf_state.lock() {
        perf.raster_fps = raster_frame_buffer.fps();
    }
    true
}

fn send_gpu_submitted(
    scheduler_tx: &crossbeam_channel::Sender<SchedulerEvent>,
    render_complete_tx: &mpsc::Sender<()>,
    is_rendering: &Arc<AtomicBool>,
    frame_id: u64,
    epoch: u64,
    frame_time_ms: f32,
    will_present: bool,
) {
    let stats = crate::FrameStats {
        frame_time_ms,
        ..Default::default()
    };
    let _ = scheduler_tx.send(SchedulerEvent::GpuSubmitted {
        frame_id,
        epoch,
        stats,
        will_present,
    });
    let _ = render_complete_tx.send(());
    is_rendering.store(false, Ordering::Release);
}
