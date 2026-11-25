//! Easing and tweens for animations.

use std::time::{Duration, Instant};

/// Fast start, gentle stop. `t` is clamped to 0..=1.
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let u = 1.0 - t;
    1.0 - u * u * u
}

/// A value moving from `from` to `to` over `dur`, starting at `start`.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
    pub dur: Duration,
}

impl Tween {
    pub fn value(&self, now: Instant) -> f32 {
        let t = if self.dur.is_zero() {
            1.0
        } else {
            now.saturating_duration_since(self.start).as_secs_f32() / self.dur.as_secs_f32()
        };
        self.from + (self.to - self.from) * ease_out_cubic(t)
    }

    pub fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.dur
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_cubic_endpoints_and_clamp() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_out_cubic(0.5), 0.875);
        assert_eq!(ease_out_cubic(-1.0), 0.0);
        assert_eq!(ease_out_cubic(2.0), 1.0);
    }

    #[test]
    fn tween_values_over_time() {
        let t0 = Instant::now();
        let tw = Tween { from: 0.0, to: 100.0, start: t0, dur: Duration::from_millis(200) };
        assert_eq!(tw.value(t0), 0.0);
        assert_eq!(tw.value(t0 + Duration::from_millis(200)), 100.0);
        assert_eq!(tw.value(t0 + Duration::from_millis(100)), 87.5);
        assert_eq!(tw.value(t0 + Duration::from_secs(1)), 100.0);
        assert!(!tw.done(t0 + Duration::from_millis(199)));
        assert!(tw.done(t0 + Duration::from_millis(200)));
    }
}
