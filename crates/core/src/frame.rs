//! Decides when a frame needs drawing. Idle means no frames at all.

use std::time::{Duration, Instant};

/// How often to wake while an animation runs. Swapping buffers waits for
/// the screen anyway, so this only bounds how long the loop sleeps.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Debug, Default)]
pub struct FrameScheduler {
    pending: bool,
    anim_until: Option<Instant>,
}

impl FrameScheduler {
    pub fn new() -> FrameScheduler {
        FrameScheduler::default()
    }

    /// Something changed: draw one frame.
    pub fn request_redraw(&mut self) {
        self.pending = true;
    }

    /// Keep drawing until `until`, plus one last frame at or after it.
    pub fn animate_until(&mut self, until: Instant) {
        self.anim_until = Some(match self.anim_until {
            Some(cur) if cur > until => cur,
            _ => until,
        });
    }

    pub fn is_animating(&self) -> bool {
        self.anim_until.is_some()
    }

    pub fn should_draw(&self, _now: Instant) -> bool {
        self.pending || self.anim_until.is_some()
    }

    pub fn frame_drawn(&mut self, now: Instant) {
        self.pending = false;
        if matches!(self.anim_until, Some(until) if now >= until) {
            self.anim_until = None;
        }
    }

    /// `None` means sleep until the next input or system event.
    pub fn next_wake(&self, _now: Instant) -> Option<Duration> {
        if self.pending {
            Some(Duration::ZERO)
        } else if self.anim_until.is_some() {
            Some(FRAME_INTERVAL)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_scheduler_sleeps() {
        let s = FrameScheduler::new();
        let t = Instant::now();
        assert!(!s.should_draw(t));
        assert_eq!(s.next_wake(t), None);
    }

    #[test]
    fn one_request_gives_exactly_one_frame() {
        let mut s = FrameScheduler::new();
        let t = Instant::now();
        s.request_redraw();
        assert!(s.should_draw(t));
        assert_eq!(s.next_wake(t), Some(Duration::ZERO));
        s.frame_drawn(t);
        assert!(!s.should_draw(t));
        assert_eq!(s.next_wake(t), None);
    }

    #[test]
    fn animation_draws_until_deadline_then_stops() {
        let mut s = FrameScheduler::new();
        let t = Instant::now();
        s.animate_until(t + Duration::from_millis(100));
        assert!(s.should_draw(t));
        s.frame_drawn(t);
        assert_eq!(s.next_wake(t), Some(FRAME_INTERVAL));
        let after = t + Duration::from_millis(101);
        assert!(s.should_draw(after)); // one last frame at the end state
        s.frame_drawn(after);
        assert!(!s.should_draw(after));
        assert_eq!(s.next_wake(after), None);
    }

    #[test]
    fn reports_animating_only_until_deadline_frame() {
        let mut s = FrameScheduler::new();
        let t = Instant::now();
        assert!(!s.is_animating());
        s.animate_until(t + Duration::from_millis(50));
        assert!(s.is_animating());
        s.frame_drawn(t + Duration::from_millis(60));
        assert!(!s.is_animating());
    }

    #[test]
    fn later_deadline_wins() {
        let mut s = FrameScheduler::new();
        let t = Instant::now();
        s.animate_until(t + Duration::from_millis(300));
        s.animate_until(t + Duration::from_millis(100));
        s.frame_drawn(t + Duration::from_millis(150));
        assert_eq!(s.next_wake(t + Duration::from_millis(150)), Some(FRAME_INTERVAL));
    }
}
