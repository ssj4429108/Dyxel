use std::collections::VecDeque;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameResultClass {
    OnTime,
    Late,
    Dropped,
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
}

pub struct FrameTimeline {
    recent: VecDeque<FrameRecord>,
    next_frame_id: u64,
}

impl FrameTimeline {
    pub fn new() -> Self {
        Self {
            recent: VecDeque::new(),
            next_frame_id: 1,
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

    pub fn recent(&self) -> &VecDeque<FrameRecord> {
        &self.recent
    }

    pub fn recent_mut(&mut self) -> &mut VecDeque<FrameRecord> {
        &mut self.recent
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

        timeline.record_token(frame_id, 7, now, now);
        timeline.mark_render_started(frame_id, now);
        timeline.mark_render_completed(frame_id, now);

        let recent = timeline.recent();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].epoch, 7);
    }
}
