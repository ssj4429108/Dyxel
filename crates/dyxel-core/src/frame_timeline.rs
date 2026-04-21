use std::collections::VecDeque;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameResultClass {
    /// Frame rendered and presented within budget.
    OnTime,
    /// Frame completed but exceeded target duration (cadence miss).
    MissedCadence,
    /// VBlank arrived but scheduler was Idle — no new content to display.
    SkippedIdle,
    /// VBlank arrived but cadence divisor suppressed this tick.
    SkippedDivisor,
    /// VBlank arrived but a frame was already in flight.
    SkippedInFlight,
}

#[derive(Debug, Clone)]
pub struct FrameRecord {
    pub frame_id: u64,
    pub epoch: u64,
    pub token_issued_at: Instant,
    pub vblank_at: Instant,
    pub render_started_at: Option<Instant>,
    pub render_completed_at: Option<Instant>,
    pub frame_result: Option<FrameResultClass>,
    pub presented_at: Option<Instant>,
    /// Target frame duration in ms (from cadence decision). Used to classify
    /// OnTime vs MissedCadence.
    pub target_frame_duration_ms: f64,
    /// If this record represents a skipped VBlank rather than a real token.
    pub is_skipped: bool,
    /// Display refresh rate at the time of this frame (Hz).
    pub display_hz: f64,
    /// Cadence divisor at the time of this frame.
    pub divisor: u32,
    /// Effective refresh rate (display_hz / divisor).
    pub effective_hz: f64,
    /// Epochs dropped between the last presented frame and this one.
    pub dropped_epochs_since_last_present: u64,
}

pub struct FrameTimeline {
    recent: VecDeque<FrameRecord>,
    next_frame_id: u64,
    /// Running counters for diagnostics.
    on_time_count: u64,
    missed_cadence_count: u64,
    skipped_idle_count: u64,
    skipped_divisor_count: u64,
    skipped_in_flight_count: u64,
    /// Cumulative count of dropped epochs across all presented frames.
    dropped_epoch_count: u64,
}

impl FrameTimeline {
    pub fn new() -> Self {
        Self {
            recent: VecDeque::new(),
            next_frame_id: 1,
            on_time_count: 0,
            missed_cadence_count: 0,
            skipped_idle_count: 0,
            skipped_divisor_count: 0,
            skipped_in_flight_count: 0,
            dropped_epoch_count: 0,
        }
    }

    pub fn next_frame_id(&mut self) -> u64 {
        let id = self.next_frame_id;
        self.next_frame_id += 1;
        id
    }

    pub fn record_token(
        &mut self,
        frame_id: u64,
        epoch: u64,
        vblank_at: Instant,
        token_issued_at: Instant,
        target_frame_duration_ms: f64,
        display_hz: f64,
        divisor: u32,
        effective_hz: f64,
        dropped_epochs_since_last_present: u64,
    ) {
        self.recent.push_back(FrameRecord {
            frame_id,
            epoch,
            token_issued_at,
            vblank_at,
            render_started_at: None,
            render_completed_at: None,
            frame_result: None,
            presented_at: None,
            target_frame_duration_ms,
            is_skipped: false,
            display_hz,
            divisor,
            effective_hz,
            dropped_epochs_since_last_present,
        });
        self.dropped_epoch_count += dropped_epochs_since_last_present;
    }

    /// Record a VBlank that did not produce a frame token.
    pub fn record_skipped_vblank(
        &mut self,
        vblank_at: Instant,
        result_class: FrameResultClass,
        display_hz: f64,
        divisor: u32,
        effective_hz: f64,
    ) {
        match result_class {
            FrameResultClass::SkippedIdle => self.skipped_idle_count += 1,
            FrameResultClass::SkippedDivisor => self.skipped_divisor_count += 1,
            FrameResultClass::SkippedInFlight => self.skipped_in_flight_count += 1,
            _ => {}
        }
        self.recent.push_back(FrameRecord {
            frame_id: 0,
            epoch: 0,
            token_issued_at: vblank_at,
            vblank_at,
            render_started_at: None,
            render_completed_at: None,
            frame_result: Some(result_class),
            presented_at: None,
            target_frame_duration_ms: 0.0,
            is_skipped: true,
            display_hz,
            divisor,
            effective_hz,
            dropped_epochs_since_last_present: 0,
        });
    }

    pub fn mark_render_started(&mut self, frame_id: u64, at: Instant) {
        if let Some(rec) = self.recent.iter_mut().find(|r| r.frame_id == frame_id) {
            rec.render_started_at = Some(at);
        }
    }

    pub fn mark_render_completed(&mut self, frame_id: u64, at: Instant) {
        if let Some(rec) = self.recent.iter_mut().find(|r| r.frame_id == frame_id) {
            rec.render_completed_at = Some(at);
        }
    }

    /// Classify the frame result based on timing. Should be called after
    /// render_completed_at is set. Returns the classified result if found.
    pub fn classify_frame_result(&mut self, frame_id: u64) -> Option<FrameResultClass> {
        if let Some(rec) = self.recent.iter_mut().find(|r| r.frame_id == frame_id) {
            if rec.is_skipped {
                return None;
            }
            let result = if let (Some(started), Some(completed)) =
                (rec.render_started_at, rec.render_completed_at)
            {
                let render_duration_ms = completed.duration_since(started).as_secs_f64() * 1000.0;
                if render_duration_ms > rec.target_frame_duration_ms {
                    self.missed_cadence_count += 1;
                    FrameResultClass::MissedCadence
                } else {
                    self.on_time_count += 1;
                    FrameResultClass::OnTime
                }
            } else {
                self.missed_cadence_count += 1;
                FrameResultClass::MissedCadence
            };
            rec.frame_result = Some(result);
            return Some(result);
        }
        None
    }

    pub fn recent(&self) -> &VecDeque<FrameRecord> {
        &self.recent
    }

    pub fn recent_mut(&mut self) -> &mut VecDeque<FrameRecord> {
        &mut self.recent
    }

    /// Diagnostics: return counts of each result class.
    /// Cumulative dropped epochs across all presented frames.
    pub fn dropped_epoch_count(&self) -> u64 {
        self.dropped_epoch_count
    }

    pub fn result_counts(&self) -> FrameResultCounts {
        FrameResultCounts {
            on_time: self.on_time_count,
            missed_cadence: self.missed_cadence_count,
            skipped_idle: self.skipped_idle_count,
            skipped_divisor: self.skipped_divisor_count,
            skipped_in_flight: self.skipped_in_flight_count,
        }
    }

    /// Trim old records to keep memory bounded.
    pub fn trim(&mut self, max_len: usize) {
        while self.recent.len() > max_len {
            self.recent.pop_front();
        }
    }

    /// Export all records as Chrome Trace Event Format JSON.
    /// Returns a JSON string containing an array of trace events.
    /// Timestamps are microseconds since the first event in the timeline.
    pub fn export_chrome_trace(&self) -> String {
        let baseline = self.recent.front().map(|r| r.vblank_at.min(r.token_issued_at));
        let base_to_us = |t: Instant| -> u64 {
            match baseline {
                Some(b) => t.duration_since(b).as_micros() as u64,
                None => 0,
            }
        };

        let mut out = String::with_capacity(self.recent.len() * 256);
        out.push('[');

        for (i, rec) in self.recent.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            if rec.is_skipped {
                // Instant event for skipped VBlank
                let name = match rec.frame_result {
                    Some(FrameResultClass::SkippedIdle) => "skipped_idle",
                    Some(FrameResultClass::SkippedDivisor) => "skipped_divisor",
                    Some(FrameResultClass::SkippedInFlight) => "skipped_inflight",
                    _ => "skipped",
                };
                let ts_us = base_to_us(rec.vblank_at);
                out.push_str("{\"name\":\"");
                out.push_str(name);
                out.push_str("\",\"ph\":\"i\",\"ts\":");
                out.push_str(&ts_us.to_string());
                out.push_str(",\"s\":\"g\",\"args\":{\"display_hz\":");
                out.push_str(&rec.display_hz.to_string());
                out.push_str(",\"divisor\":");
                out.push_str(&rec.divisor.to_string());
                out.push_str(",\"effective_hz\":");
                out.push_str(&format!("{:.2}", rec.effective_hz));
                out.push_str("}}");
            } else {
                // Complete event for frame (from token issue to completion)
                let name = match rec.frame_result {
                    Some(FrameResultClass::OnTime) => "frame_ontime",
                    Some(FrameResultClass::MissedCadence) => "frame_missed",
                    _ => "frame",
                };
                let ts_us = base_to_us(rec.token_issued_at);
                let dur_us = rec
                    .render_completed_at
                    .map(|c| c.duration_since(rec.token_issued_at).as_micros() as f64)
                    .unwrap_or(0.0);
                out.push_str("{\"name\":\"");
                out.push_str(name);
                out.push_str("\",\"ph\":\"X\",\"ts\":");
                out.push_str(&ts_us.to_string());
                out.push_str(",\"dur\":");
                out.push_str(&format!("{:.0}", dur_us));
                out.push_str(",\"args\":{\"frame_id\":");
                out.push_str(&rec.frame_id.to_string());
                out.push_str(",\"epoch\":");
                out.push_str(&rec.epoch.to_string());
                out.push_str(",\"display_hz\":");
                out.push_str(&rec.display_hz.to_string());
                out.push_str(",\"divisor\":");
                out.push_str(&rec.divisor.to_string());
                out.push_str(",\"effective_hz\":");
                out.push_str(&format!("{:.2}", rec.effective_hz));
                out.push_str(",\"target_ms\":");
                out.push_str(&rec.target_frame_duration_ms.to_string());
                out.push_str(",\"dropped_epochs\":");
                out.push_str(&rec.dropped_epochs_since_last_present.to_string());
                out.push_str("}}");
            }
        }

        out.push(']');
        out
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FrameResultCounts {
    pub on_time: u64,
    pub missed_cadence: u64,
    pub skipped_idle: u64,
    pub skipped_divisor: u64,
    pub skipped_in_flight: u64,
}

impl FrameResultCounts {
    pub fn total_presented(&self) -> u64 {
        self.on_time + self.missed_cadence
    }

    pub fn total_skipped(&self) -> u64 {
        self.skipped_idle + self.skipped_divisor + self.skipped_in_flight
    }

    pub fn missed_cadence_rate(&self) -> f64 {
        let total = self.total_presented();
        if total == 0 {
            0.0
        } else {
            self.missed_cadence as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_records_frame_lifecycle() {
        let mut timeline = FrameTimeline::new();
        let frame_id = timeline.next_frame_id();
        let now = std::time::Instant::now();

        timeline.record_token(frame_id, 7, now, now, 16.67, 60.0, 1, 60.0, 0);
        timeline.mark_render_started(frame_id, now);
        timeline.mark_render_completed(frame_id, now);
        timeline.classify_frame_result(frame_id);

        let recent = timeline.recent();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].epoch, 7);
        assert_eq!(recent[0].frame_result, Some(FrameResultClass::OnTime));
    }

    #[test]
    fn timeline_classifies_missed_cadence() {
        let mut timeline = FrameTimeline::new();
        let frame_id = timeline.next_frame_id();
        let now = std::time::Instant::now();

        timeline.record_token(frame_id, 1, now, now, 16.67, 60.0, 1, 60.0, 0);
        timeline.mark_render_started(frame_id, now);
        // Simulate render taking 25ms (exceeds 16.67ms target)
        timeline.mark_render_completed(frame_id, now + std::time::Duration::from_millis(25));
        timeline.classify_frame_result(frame_id);

        assert_eq!(
            timeline.recent()[0].frame_result,
            Some(FrameResultClass::MissedCadence)
        );
        assert!(timeline.result_counts().missed_cadence_rate() > 0.0);
    }

    #[test]
    fn timeline_accumulates_dropped_epoch_count() {
        let mut timeline = FrameTimeline::new();
        let now = std::time::Instant::now();

        timeline.record_token(1, 1, now, now, 16.67, 60.0, 1, 60.0, 2);
        timeline.record_token(2, 4, now, now, 16.67, 60.0, 1, 60.0, 3);

        assert_eq!(timeline.dropped_epoch_count(), 5);
    }

    #[test]
    fn timeline_records_skipped_vblank() {
        let mut timeline = FrameTimeline::new();
        let now = std::time::Instant::now();

        timeline.record_skipped_vblank(now, FrameResultClass::SkippedDivisor, 60.0, 2, 30.0);

        let recent = timeline.recent();
        assert_eq!(recent.len(), 1);
        assert!(recent[0].is_skipped);
        assert_eq!(recent[0].frame_result, Some(FrameResultClass::SkippedDivisor));

        let counts = timeline.result_counts();
        assert_eq!(counts.skipped_divisor, 1);
    }

    #[test]
    fn timeline_exports_chrome_trace() {
        let mut timeline = FrameTimeline::new();
        let now = std::time::Instant::now();

        timeline.record_skipped_vblank(now, FrameResultClass::SkippedDivisor, 60.0, 2, 30.0);
        let frame_id = timeline.next_frame_id();
        timeline.record_token(frame_id, 1, now, now, 16.67, 60.0, 1, 60.0, 0);
        timeline.mark_render_started(frame_id, now);
        timeline.mark_render_completed(frame_id, now + std::time::Duration::from_millis(10));
        timeline.classify_frame_result(frame_id);

        let json = timeline.export_chrome_trace();
        assert!(json.starts_with('['));
        assert!(json.ends_with(']'));
        assert!(json.contains("skipped_divisor"));
        assert!(json.contains("frame_ontime"));
        assert!(json.contains("\"display_hz\":60"));
        assert!(json.contains("\"divisor\":2"));
    }
}
