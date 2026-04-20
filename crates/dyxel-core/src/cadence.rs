use std::time::{Duration, Instant};

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

pub struct CadenceGovernor {
    display_hz: f64,
    divisor: u32,
    supported_divisors: Vec<u32>,
    vblank_counter: u64,
    frame_time_window: Vec<f64>,
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
            frame_time_window: Vec::with_capacity(10),
        }
    }

    pub fn supported_divisors(&self) -> &[u32] {
        &self.supported_divisors
    }

    pub fn set_divisor_for_test(&mut self, divisor: u32) {
        self.divisor = divisor;
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

    pub fn record_frame_duration(&mut self, frame_time_ms: f64) {
        self.frame_time_window.push(frame_time_ms);
        if self.frame_time_window.len() > 10 {
            self.frame_time_window.remove(0);
        }
        self.evaluate_divisor();
    }

    fn evaluate_divisor(&mut self) {
        if self.frame_time_window.len() < 5 {
            return;
        }
        let avg = self.frame_time_window.iter().sum::<f64>() / self.frame_time_window.len() as f64;
        let target_ms = 1000.0 / self.display_hz;
        let current_target_ms = target_ms * self.divisor as f64;

        // Upgrade (lower divisor = higher FPS) if we have headroom
        if self.divisor > 1 && avg < current_target_ms * 0.7 {
            let next = self
                .supported_divisors
                .iter()
                .find(|&&d| d < self.divisor)
                .copied();
            if let Some(d) = next {
                log::debug!("CadenceGovernor: upgrading divisor {} -> {}", self.divisor, d);
                self.divisor = d;
                self.frame_time_window.clear();
            }
        }
        // Downgrade (higher divisor = lower FPS) if we're missing budget
        else if avg > current_target_ms * 1.15 {
            let next = self
                .supported_divisors
                .iter()
                .find(|&&d| d > self.divisor)
                .copied();
            if let Some(d) = next {
                log::info!(
                    "CadenceGovernor: downgrading divisor {} -> {} (avg {:.2}ms > budget {:.2}ms)",
                    self.divisor,
                    d,
                    avg,
                    current_target_ms
                );
                self.divisor = d;
                self.frame_time_window.clear();
            }
        }
    }
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
}
