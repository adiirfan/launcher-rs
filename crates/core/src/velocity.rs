//! Finger speed over the last moment of a gesture.
//!
//! Android's "finger lifted" event often repeats the last position, so the
//! speed of only the final step would read as zero after a quick flick.
//! Measuring across the last 100 ms keeps the flick.

use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_millis(100);

#[derive(Debug, Default, Clone)]
pub struct Velocity {
    samples: Vec<(Instant, f32, f32)>,
}

impl Velocity {
    pub fn new() -> Velocity {
        Velocity { samples: Vec::with_capacity(16) }
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn add(&mut self, t: Instant, x: f32, y: f32) {
        self.samples.push((t, x, y));
        let cutoff = t.checked_sub(WINDOW);
        if let Some(cutoff) = cutoff {
            self.samples.retain(|s| s.0 >= cutoff);
        }
    }

    /// (x, y) in px per ms; zero with fewer than two samples in the window.
    pub fn get(&self) -> (f32, f32) {
        let (Some(first), Some(last)) = (self.samples.first(), self.samples.last()) else { return (0.0, 0.0) };
        let dt = (last.0 - first.0).as_secs_f32() * 1000.0;
        if dt <= 0.0 {
            return (0.0, 0.0);
        }
        ((last.1 - first.1) / dt, (last.2 - first.2) / dt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_last_point_keeps_the_flick() {
        let t0 = Instant::now();
        let mut v = Velocity::new();
        v.add(t0, 0.0, 800.0);
        v.add(t0 + Duration::from_millis(20), 0.0, 760.0);
        v.add(t0 + Duration::from_millis(40), 0.0, 720.0);
        v.add(t0 + Duration::from_millis(48), 0.0, 720.0);
        let (_, vy) = v.get();
        assert!((vy - (-80.0 / 48.0)).abs() < 0.01, "{vy}");
    }

    #[test]
    fn old_samples_drop_out() {
        let t0 = Instant::now();
        let mut v = Velocity::new();
        v.add(t0, 800.0, 0.0);
        v.add(t0 + Duration::from_millis(300), 368.0, 0.0);
        v.add(t0 + Duration::from_millis(600), 368.0, 0.0);
        assert_eq!(v.get(), (0.0, 0.0));
    }

    #[test]
    fn one_sample_is_still() {
        let mut v = Velocity::new();
        v.add(Instant::now(), 5.0, 5.0);
        assert_eq!(v.get(), (0.0, 0.0));
        v.clear();
        assert_eq!(v.get(), (0.0, 0.0));
    }
}
