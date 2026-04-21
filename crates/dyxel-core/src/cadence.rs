use std::time::{Duration, Instant};

/// Number of recent frames used for downgrade decisions (fast response).
const SHORT_WINDOW: usize = 8;
/// Number of recent frames used for upgrade decisions (stable response).
const LONG_WINDOW: usize = 120;
/// Minimum time between divisor changes to avoid oscillation.
const MIN_RESIDENCY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy)]
pub struct CadenceDecision {
    pub should_present_this_tick: bool,
    pub divisor: u32,
    pub effective_hz: f64,
    pub target_frame_duration: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct CadenceInfo {
    pub display_hz: f64,
    pub divisor: u32,
    pub effective_hz: f64,
    pub target_frame_duration: Duration,
    pub expected_present_time: Instant,
}

/// Per-frame input to the CadenceGovernor.
#[derive(Debug, Clone, Copy)]
pub struct GovernorFrameRecord {
    /// Total frame time from token issue to render completion (ms).
    pub frame_time_ms: f64,
    /// GPU time if available (ms).
    pub gpu_time_ms: Option<f64>,
    /// Whether this frame missed its cadence target.
    pub missed_cadence: bool,
}

pub struct CadenceGovernor {
    display_hz: f64,
    divisor: u32,
    supported_divisors: Vec<u32>,
    vblank_counter: u64,
    /// Short window for downgrade decisions (fast response to pressure).
    short_window: Vec<GovernorFrameRecord>,
    /// Long window for upgrade decisions (stable headroom detection).
    long_window: Vec<GovernorFrameRecord>,
    /// When the divisor was last changed. Used to enforce MIN_RESIDENCY.
    last_divisor_change: Instant,
    /// Consecutive SKIPPED_IN_FLIGHT count. Reset on successful present.
    consecutive_skipped_in_flight: u32,
}

impl CadenceGovernor {
    pub fn new(display_hz: f64) -> Self {
        let supported_divisors = if display_hz <= 61.0 {
            vec![1, 2]
        } else if display_hz <= 91.0 {
            vec![1, 2, 3]
        } else {
            vec![1, 2, 3, 4]
        };
        Self {
            display_hz,
            divisor: 1,
            supported_divisors,
            vblank_counter: 0,
            short_window: Vec::with_capacity(SHORT_WINDOW),
            long_window: Vec::with_capacity(LONG_WINDOW),
            last_divisor_change: Instant::now(),
            consecutive_skipped_in_flight: 0,
        }
    }

    pub fn supported_divisors(&self) -> &[u32] {
        &self.supported_divisors
    }

    pub fn set_divisor_for_test(&mut self, divisor: u32) {
        self.divisor = divisor;
    }

    /// Reset the residency timer so tests can immediately evaluate divisor changes.
    #[cfg(test)]
    pub fn reset_residency_for_test(&mut self) {
        self.last_divisor_change = Instant::now() - MIN_RESIDENCY - Duration::from_millis(1);
    }

    pub fn on_vblank(&mut self, _now: Instant) -> CadenceDecision {
        self.vblank_counter += 1;
        let should_present_this_tick = (self.vblank_counter - 1) % self.divisor as u64 == 0;
        let effective_hz = self.display_hz / self.divisor as f64;
        CadenceDecision {
            should_present_this_tick,
            divisor: self.divisor,
            effective_hz,
            target_frame_duration: Duration::from_secs_f64(1.0 / effective_hz),
        }
    }

    pub fn current_decision(&self) -> CadenceDecision {
        let effective_hz = self.display_hz / self.divisor as f64;
        CadenceDecision {
            should_present_this_tick: true,
            divisor: self.divisor,
            effective_hz,
            target_frame_duration: Duration::from_secs_f64(1.0 / effective_hz),
        }
    }

    pub fn info(&self) -> CadenceInfo {
        let effective_hz = self.display_hz / self.divisor as f64;
        CadenceInfo {
            display_hz: self.display_hz,
            divisor: self.divisor,
            effective_hz,
            target_frame_duration: Duration::from_secs_f64(1.0 / effective_hz),
            expected_present_time: Instant::now(),
        }
    }

    /// Record a skipped-in-flight VBlank. This increments the consecutive
    /// counter used by the downgrade rule.
    pub fn record_skipped_in_flight(&mut self) {
        self.consecutive_skipped_in_flight += 1;
        self.evaluate_divisor();
    }

    pub fn record_frame(&mut self, record: GovernorFrameRecord) {
        if record.frame_time_ms <= 0.0 {
            return;
        }
        // Reset skipped-in-flight counter on any successful frame completion.
        self.consecutive_skipped_in_flight = 0;

        self.short_window.push(record);
        if self.short_window.len() > SHORT_WINDOW {
            self.short_window.remove(0);
        }
        self.long_window.push(record);
        if self.long_window.len() > LONG_WINDOW {
            self.long_window.remove(0);
        }
        self.evaluate_divisor();
    }

    fn evaluate_divisor(&mut self) {
        // Enforce min residency: do not change divisor more often than every 2s.
        if self.last_divisor_change.elapsed() < MIN_RESIDENCY {
            return;
        }

        let effective_hz = self.display_hz / self.divisor as f64;
        let target_frame_ms = 1000.0 / effective_hz;

        // --- Downgrade rules (design doc §13.4) ---
        // 1. short_missed_rate >= 0.25
        // 2. short_p95_total_ms >= downgrade_pressure (0.85 * target)
        // 3. short_p95_gpu_ms >= 0.75 * target
        // 4. consecutive_skipped_in_flight >= 3

        // Rule 4 (skipped-in-flight) can trigger even with insufficient window data.
        if self.consecutive_skipped_in_flight >= 3 {
            if let Some(d) = self
                .supported_divisors
                .iter()
                .find(|&&d| d > self.divisor)
                .copied()
            {
                log::info!(
                    "CadenceGovernor: downgrading divisor {} -> {} (skipped_in_flight={})",
                    self.divisor,
                    d,
                    self.consecutive_skipped_in_flight,
                );
                self.divisor = d;
                self.last_divisor_change = Instant::now();
                return;
            }
        }

        if self.short_window.len() >= SHORT_WINDOW / 2 {
            let short_missed_rate = self
                .short_window
                .iter()
                .filter(|r| r.missed_cadence)
                .count() as f64
                / self.short_window.len() as f64;
            let short_p95_total = percentile(&self.short_window, |r| r.frame_time_ms, 0.95);
            let short_p95_gpu = percentile(
                &self.short_window,
                |r| r.gpu_time_ms.unwrap_or(0.0),
                0.95,
            );

            let downgrade_pressure = 0.85 * target_frame_ms;

            let should_downgrade = short_missed_rate >= 0.25
                || short_p95_total >= downgrade_pressure
                || short_p95_gpu >= 0.75 * target_frame_ms;

            if should_downgrade {
                if let Some(d) = self
                    .supported_divisors
                    .iter()
                    .find(|&&d| d > self.divisor)
                    .copied()
                {
                    log::info!(
                        "CadenceGovernor: downgrading divisor {} -> {} (missed={:.0}% p95_total={:.2}ms p95_gpu={:.2}ms skipped_in_flight={})",
                        self.divisor,
                        d,
                        short_missed_rate * 100.0,
                        short_p95_total,
                        short_p95_gpu,
                        self.consecutive_skipped_in_flight,
                    );
                    self.divisor = d;
                    self.last_divisor_change = Instant::now();
                    return;
                }
            }
        }

        // --- Upgrade rules (design doc §13.5) ---
        // All must be true:
        // 1. long_missed_rate <= 0.01
        // 2. long_p95_total_ms <= upgrade_headroom (0.60 * target)
        // 3. long_p95_gpu_ms <= 0.50 * target
        // 4. No significant skipped_in_flight in long window

        if self.divisor > 1 && self.long_window.len() >= LONG_WINDOW / 2 {
            let long_missed_rate = self
                .long_window
                .iter()
                .filter(|r| r.missed_cadence)
                .count() as f64
                / self.long_window.len() as f64;
            let long_p95_total = percentile(&self.long_window, |r| r.frame_time_ms, 0.95);
            let long_p95_gpu = percentile(
                &self.long_window,
                |r| r.gpu_time_ms.unwrap_or(0.0),
                0.95,
            );

            let upgrade_headroom = 0.60 * target_frame_ms;

            let should_upgrade = long_missed_rate <= 0.01
                && long_p95_total <= upgrade_headroom
                && long_p95_gpu <= 0.50 * target_frame_ms
                && self.consecutive_skipped_in_flight == 0;

            if should_upgrade {
                if let Some(d) = self
                    .supported_divisors
                    .iter()
                    .find(|&&d| d < self.divisor)
                    .copied()
                {
                    log::info!(
                        "CadenceGovernor: upgrading divisor {} -> {} (missed={:.0}% p95_total={:.2}ms p95_gpu={:.2}ms)",
                        self.divisor,
                        d,
                        long_missed_rate * 100.0,
                        long_p95_total,
                        long_p95_gpu,
                    );
                    self.divisor = d;
                    self.last_divisor_change = Instant::now();
                }
            }
        }
    }
}

/// Compute the p-th percentile (0.0..1.0) of `f` applied to each element.
/// Returns 0.0 if the slice is empty.
fn percentile<T>(items: &[T], f: impl Fn(&T) -> f64, p: f64) -> f64 {
    if items.is_empty() {
        return 0.0;
    }
    let mut vals: Vec<f64> = items.iter().map(|x| f(x)).collect();
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((vals.len() - 1) as f64 * p) as usize;
    vals[idx.clamp(0, vals.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_governor_120hz_uses_integer_divisors() {
        let gov = CadenceGovernor::new(120.0);
        assert_eq!(gov.supported_divisors(), &[1, 2, 3, 4]);
    }

    #[test]
    fn cadence_governor_60hz_uses_integer_divisors() {
        let gov = CadenceGovernor::new(60.0);
        assert_eq!(gov.supported_divisors(), &[1, 2]);
    }

    #[test]
    fn cadence_governor_only_presents_on_divisor_ticks() {
        let mut gov = CadenceGovernor::new(120.0);
        gov.set_divisor_for_test(2);

        let d1 = gov.on_vblank(Instant::now());
        let d2 = gov.on_vblank(Instant::now());

        assert!(d1.should_present_this_tick);
        assert!(!d2.should_present_this_tick);
    }

    #[test]
    fn cadence_governor_downgrades_on_short_window_pressure() {
        let mut gov = CadenceGovernor::new(60.0);
        gov.reset_residency_for_test();
        // At 60Hz divisor=1, target = 16.67ms. Downgrade fires when
        // short_p95 >= 0.85 * 16.67 = 14.17ms. Inject 8 frames of 15ms
        // to trigger downgrade.
        for _ in 0..8 {
            gov.record_frame(GovernorFrameRecord {
                frame_time_ms: 15.0,
                gpu_time_ms: None,
                missed_cadence: false,
            });
        }
        assert_eq!(gov.divisor, 2);
    }

    #[test]
    fn cadence_governor_downgrades_on_missed_cadence() {
        let mut gov = CadenceGovernor::new(60.0);
        gov.reset_residency_for_test();
        // At 60Hz divisor=1. Inject 8 frames with 25% missed rate.
        for i in 0..8 {
            gov.record_frame(GovernorFrameRecord {
                frame_time_ms: 16.0,
                gpu_time_ms: None,
                missed_cadence: i % 4 == 0, // 25% missed
            });
        }
        assert_eq!(gov.divisor, 2);
    }

    #[test]
    fn cadence_governor_downgrades_on_skipped_in_flight() {
        let mut gov = CadenceGovernor::new(60.0);
        gov.reset_residency_for_test();
        // 3 consecutive skipped-in-flight triggers downgrade.
        gov.record_skipped_in_flight();
        gov.record_skipped_in_flight();
        gov.record_skipped_in_flight();
        assert_eq!(gov.divisor, 2);
    }

    #[test]
    fn cadence_governor_upgrades_on_long_window_headroom() {
        let mut gov = CadenceGovernor::new(120.0);
        gov.set_divisor_for_test(2);
        gov.reset_residency_for_test();
        // At 120Hz divisor=2, effective=60Hz, target = 16.67ms.
        // Upgrade fires when long_p95 <= 0.60 * 16.67 = 10.0ms.
        // Inject 60 frames of 8ms (above LONG_WINDOW/2 = 60).
        for _ in 0..60 {
            gov.record_frame(GovernorFrameRecord {
                frame_time_ms: 8.0,
                gpu_time_ms: None,
                missed_cadence: false,
            });
        }
        assert_eq!(gov.divisor, 1);
    }

    #[test]
    fn cadence_governor_respects_min_residency() {
        let mut gov = CadenceGovernor::new(60.0);
        gov.set_divisor_for_test(1);
        gov.reset_residency_for_test();
        // Trigger a downgrade.
        for _ in 0..8 {
            gov.record_frame(GovernorFrameRecord {
                frame_time_ms: 25.0,
                gpu_time_ms: None,
                missed_cadence: false,
            });
        }
        assert_eq!(gov.divisor, 2);
        // Immediately try to trigger upgrade with very fast frames.
        // Should be blocked by min residency.
        for _ in 0..60 {
            gov.record_frame(GovernorFrameRecord {
                frame_time_ms: 5.0,
                gpu_time_ms: None,
                missed_cadence: false,
            });
        }
        // Still 2 because min residency hasn't elapsed.
        assert_eq!(gov.divisor, 2);
    }
}
