use std::sync::Arc;
use std::time::{Duration, Instant};

const BUFFER_TIME_US: u64 = 500; // 0.5ms headroom before true VBlank
const SLEEP_THRESHOLD_MS: u64 = 2; // Use thread::sleep when >2ms away
const SPIN_COMPENSATION_US: u64 = 500; // Final 0.5ms uses spin-loop for precision
const PHASE_LOCK_EMA_ALPHA: f64 = 0.1; // Low-pass filter on measured interval
const PHASE_LOCK_GAIN: f64 = 0.2; // Correct 20% of observed drift per frame
const PHASE_LOCK_MAX_CORRECTION_RATIO: f64 = 0.3; // Cap correction at 30% of frame time
const PHASE_LOCK_OFFSET_US: i64 = -1000; // Negative offset: wake up 1ms earlier to avoid swapchain backpressure

/// Platform-provided hardware VBlank signal.
pub trait VBlankWaiter: Send + Sync {
    /// Block until the next display refresh (VBlank).
    fn wait_for_vblank(&self);
}

pub struct FramePacer {
    /// The fixed VBlank deadline that does not drift on late frames.
    target_deadline: Instant,
    target_frame_duration: Duration,
    /// Safety buffer to leave a little headroom before the true deadline.
    buffer_time: Duration,
    /// Phase lock offset in microseconds: negative values wake earlier to avoid swapchain backpressure.
    phase_offset_us: i64,
    /// Last time `wait_for_next_frame` finished (i.e. the frame actually started).
    last_wake: Option<Instant>,
    /// EMA of the drift between measured interval and target interval.
    ema_drift_secs: f64,
    /// Optional hardware VBlank waiter.
    vblank_waiter: Option<Arc<dyn VBlankWaiter>>,
}

impl FramePacer {
    pub fn new(target_fps: f64) -> Self {
        let target_frame_duration = Duration::from_secs_f64(1.0 / target_fps);
        Self {
            target_deadline: Instant::now() + target_frame_duration,
            target_frame_duration,
            buffer_time: Duration::from_micros(BUFFER_TIME_US),
            phase_offset_us: PHASE_LOCK_OFFSET_US,
            last_wake: None,
            ema_drift_secs: 0.0,
            vblank_waiter: None,
        }
    }

    pub fn set_vblank_waiter(&mut self, waiter: Arc<dyn VBlankWaiter>) {
        self.vblank_waiter = Some(waiter);
    }

    /// Spin-Sleep strategy: sleep if >2ms away, spin-loop the last 0.5ms.
    /// When a `VBlankWaiter` is registered, it is used as the primary sync
    /// source and software sleep is only used as a safety fallback.
    /// Returns the amount of time spent actively waiting.
    pub fn wait_for_next_frame(&mut self) -> Duration {
        let wait_start = Instant::now();

        // Compute the effective target, applying phase offset to wake slightly earlier
        // and leave more headroom for GPU submission, reducing swapchain backpressure.
        let base_target = self.target_deadline - self.buffer_time;
        let effective_target = if self.phase_offset_us < 0 {
            base_target - Duration::from_micros(self.phase_offset_us.abs() as u64)
        } else {
            base_target + Duration::from_micros(self.phase_offset_us as u64)
        };

        // If we have a hardware VBlank signal, use it as the primary timing source.
        if let Some(ref waiter) = self.vblank_waiter {
            // Safety sleep/spin until we're close to the VBlank, honouring the offset.
            if wait_start < effective_target {
                let remaining = effective_target - wait_start;
                if remaining > Duration::from_millis(SLEEP_THRESHOLD_MS) {
                    std::thread::sleep(remaining - Duration::from_micros(SPIN_COMPENSATION_US));
                }
                while Instant::now() < effective_target {
                    std::hint::spin_loop();
                }
            }
            // Block directly on the hardware VBlank signal. The condvar/spin cost
            // is negligible and avoids the 1ms granularity of thread::sleep.
            waiter.wait_for_vblank();
        } else {
            // Software fallback
            if wait_start < effective_target {
                let remaining = effective_target - wait_start;
                if remaining > Duration::from_millis(SLEEP_THRESHOLD_MS) {
                    std::thread::sleep(remaining - Duration::from_micros(SPIN_COMPENSATION_US));
                }
                while Instant::now() < effective_target {
                    std::hint::spin_loop();
                }
            }
        }

        let now = Instant::now();
        let pacer_wait = now.saturating_duration_since(wait_start);

        // ---- Phase Lock ----
        // Measure actual interval since last frame start and gently correct
        // the deadline so we converge on the real display refresh phase.
        if let Some(last) = self.last_wake {
            let measured = now.saturating_duration_since(last);
            let min_interval = self.target_frame_duration.mul_f64(0.5);
            let max_interval = self.target_frame_duration.mul_f64(1.5);
            if measured >= min_interval && measured <= max_interval {
                let measured_secs = measured.as_secs_f64();
                let target_secs = self.target_frame_duration.as_secs_f64();
                let drift = measured_secs - target_secs;
                self.ema_drift_secs =
                    self.ema_drift_secs * (1.0 - PHASE_LOCK_EMA_ALPHA) + drift * PHASE_LOCK_EMA_ALPHA;
            }
        }
        self.last_wake = Some(now);

        let target_secs = self.target_frame_duration.as_secs_f64();
        let correction_secs = (self.ema_drift_secs * PHASE_LOCK_GAIN)
            .clamp(-target_secs * PHASE_LOCK_MAX_CORRECTION_RATIO, target_secs * PHASE_LOCK_MAX_CORRECTION_RATIO);
        let correction = Duration::from_secs_f64(correction_secs.abs())
            .min(self.target_frame_duration.mul_f64(PHASE_LOCK_MAX_CORRECTION_RATIO));

        // Apply correction: if we're consistently late (drift > 0), pull the deadline earlier.
        // If we're consistently early (drift < 0), push it later.
        let next_deadline = if correction_secs > 0.0 {
            self.target_deadline + self.target_frame_duration - correction
        } else {
            self.target_deadline + self.target_frame_duration + correction
        };

        // Robust reset: if deadline has already passed, start fresh from now.
        self.target_deadline = if next_deadline <= now {
            now + self.target_frame_duration
        } else {
            next_deadline
        };

        pacer_wait
    }

    /// Reserved hook called after the frame is presented to the display.
    /// Currently a no-op because pacing is anchored on `wait_for_next_frame`.
    pub fn on_frame_submitted(&mut self) {}
}
