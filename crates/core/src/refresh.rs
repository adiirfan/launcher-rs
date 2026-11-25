//! Adaptive refresh: ask for the screen's fastest rate while something moves,
//! and give the request up shortly after everything is still.

use std::time::{Duration, Instant};

/// How long the fast rate is kept after the last touch or animation frame.
pub const HOLD: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateVote {
    /// The screen's fastest rate.
    Max,
    /// No preference: the phone may drop to its slowest rate.
    Idle,
}

#[derive(Debug)]
pub struct RefreshPolicy {
    current: RateVote,
    last_active: Option<Instant>,
}

impl Default for RefreshPolicy {
    fn default() -> Self {
        RefreshPolicy { current: RateVote::Idle, last_active: None }
    }
}

impl RefreshPolicy {
    pub fn new() -> RefreshPolicy {
        RefreshPolicy::default()
    }

    /// A touch (down, move or up) or a frame drawn for an animation.
    pub fn note_activity(&mut self, now: Instant) {
        self.last_active = Some(now);
    }

    fn wanted(&self, now: Instant) -> RateVote {
        match self.last_active {
            Some(t) if now.saturating_duration_since(t) < HOLD => RateVote::Max,
            _ => RateVote::Idle,
        }
    }

    /// The new vote, only when it differs from the current one.
    pub fn update(&mut self, now: Instant) -> Option<RateVote> {
        let want = self.wanted(now);
        (want != self.current).then(|| {
            self.current = want;
            want
        })
    }

    /// Time until the drop to `Idle` is due; `None` when already idle.
    pub fn next_check(&self, now: Instant) -> Option<Duration> {
        if self.current == RateVote::Idle {
            return None;
        }
        let t = self.last_active?;
        Some((t + HOLD).saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn starts_idle_with_nothing_to_change() {
        let mut p = RefreshPolicy::new();
        let t = Instant::now();
        assert_eq!(p.update(t), None);
        assert_eq!(p.next_check(t), None);
    }

    #[test]
    fn activity_votes_max_once() {
        let mut p = RefreshPolicy::new();
        let t = Instant::now();
        p.note_activity(t);
        assert_eq!(p.update(t), Some(RateVote::Max));
        assert_eq!(p.update(t + MS(10)), None);
    }

    #[test]
    fn drops_to_idle_after_hold() {
        let mut p = RefreshPolicy::new();
        let t = Instant::now();
        p.note_activity(t);
        p.update(t);
        assert_eq!(p.update(t + MS(499)), None);
        assert_eq!(p.next_check(t + MS(499)), Some(MS(1)));
        assert_eq!(p.update(t + HOLD), Some(RateVote::Idle));
        assert_eq!(p.next_check(t + HOLD), None);
        p.note_activity(t + MS(900));
        assert_eq!(p.update(t + MS(900)), Some(RateVote::Max));
    }

    #[test]
    fn continued_activity_extends_the_hold() {
        let mut p = RefreshPolicy::new();
        let t = Instant::now();
        p.note_activity(t);
        p.update(t);
        p.note_activity(t + MS(400));
        assert_eq!(p.update(t + MS(600)), None);
        assert_eq!(p.update(t + MS(900)), Some(RateVote::Idle));
    }
}
