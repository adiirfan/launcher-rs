//! Decides when a finger held still has become a long-press.

use std::time::{Duration, Instant};

pub const LONG_PRESS: Duration = Duration::from_millis(450);

pub struct LongPress {
    delay: Duration,
    slop: f32,
    down: Option<(f32, f32, Instant)>,
}

impl LongPress {
    pub fn new(delay: Duration, slop_px: f32) -> LongPress {
        LongPress { delay, slop: slop_px, down: None }
    }

    pub fn down(&mut self, x: f32, y: f32, t: Instant) {
        self.down = Some((x, y, t));
    }

    /// A finger that drifts past the slop is a swipe or a drag, never a
    /// long-press, even if it comes back.
    pub fn moved(&mut self, x: f32, y: f32) {
        if let Some((x0, y0, _)) = self.down {
            if (x - x0).hypot(y - y0) > self.slop {
                self.down = None;
            }
        }
    }

    pub fn up(&mut self) {
        self.down = None;
    }

    pub fn cancel(&mut self) {
        self.down = None;
    }

    /// True once, the first time this is asked at or after the delay.
    pub fn fire(&mut self, now: Instant) -> bool {
        match self.down {
            Some((_, _, t)) if now.saturating_duration_since(t) >= self.delay => {
                self.down = None;
                true
            }
            _ => false,
        }
    }

    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        let (_, _, t) = self.down?;
        Some(self.delay.saturating_sub(now.saturating_duration_since(t)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn fires_once_after_the_delay() {
        let t0 = Instant::now();
        let mut lp = LongPress::new(LONG_PRESS, 24.0);
        lp.down(100.0, 100.0, t0);
        assert!(!lp.fire(t0 + ms(449)));
        lp.moved(110.0, 95.0);
        assert!(lp.fire(t0 + ms(450)));
        assert!(!lp.fire(t0 + ms(900)));
        assert_eq!(lp.next_wake(t0 + ms(900)), None);
    }

    #[test]
    fn moving_past_the_slop_cancels() {
        let t0 = Instant::now();
        let mut lp = LongPress::new(LONG_PRESS, 24.0);
        lp.down(100.0, 100.0, t0);
        lp.moved(125.0, 100.0);
        lp.moved(100.0, 100.0);
        assert!(!lp.fire(t0 + ms(500)));
    }

    #[test]
    fn lifting_or_cancel_stops_it() {
        let t0 = Instant::now();
        let mut lp = LongPress::new(LONG_PRESS, 24.0);
        lp.down(0.0, 0.0, t0);
        lp.up();
        assert!(!lp.fire(t0 + ms(500)));
        lp.down(0.0, 0.0, t0);
        lp.cancel();
        assert!(!lp.fire(t0 + ms(500)));
    }

    #[test]
    fn next_wake_counts_down() {
        let t0 = Instant::now();
        let mut lp = LongPress::new(LONG_PRESS, 24.0);
        assert_eq!(lp.next_wake(t0), None);
        lp.down(0.0, 0.0, t0);
        assert_eq!(lp.next_wake(t0 + ms(100)), Some(ms(350)));
        assert_eq!(lp.next_wake(t0 + ms(600)), Some(Duration::ZERO));
    }
}
