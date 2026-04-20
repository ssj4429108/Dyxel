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
        Self { display_hz, divisor: 1, supported_divisors, vblank_counter: 0 }
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
