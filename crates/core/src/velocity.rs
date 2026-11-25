//! Finger speed at the moment a gesture ends.
//!
//! Works the way Android's own lists do: fit a gentle curve through the
//! last 100 ms of touches and read its slope at the newest one. A flick
//! speeds up until the finger leaves the glass, so a plain average over
//! the window would read it at about half its real speed.
//!
//! The "finger lifted" event is only used to notice a finger that stopped.

use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_millis(100);
/// A finger that hasn't moved for this long has stopped.
const STOPPED: Duration = Duration::from_millis(40);
const MAX_SAMPLES: usize = 20;

#[derive(Debug, Default, Clone)]
pub struct Velocity {
    samples: Vec<(Instant, f32, f32)>,
}

impl Velocity {
    pub fn new() -> Velocity {
        Velocity { samples: Vec::with_capacity(MAX_SAMPLES + 1) }
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn add(&mut self, t: Instant, x: f32, y: f32) {
        if let Some(&(last_t, lx, ly)) = self.samples.last() {
            if t.saturating_duration_since(last_t) > STOPPED {
                self.samples.clear();
            } else if (lx, ly) == (x, y) {
                return;
            }
        }
        self.samples.push((t, x, y));
        if let Some(cutoff) = t.checked_sub(WINDOW) {
            self.samples.retain(|s| s.0 >= cutoff);
        }
        if self.samples.len() > MAX_SAMPLES {
            self.samples.remove(0);
        }
    }

    /// The finger left the glass at `t`. Its position then is not used:
    /// Android's guess for the last frame often overshoots, so the lift
    /// lands a little behind it and would read as a sudden stop.
    pub fn lift(&mut self, t: Instant) {
        if self.samples.last().is_some_and(|s| t.saturating_duration_since(s.0) > STOPPED) {
            self.samples.clear();
        }
    }

    /// (x, y) in px per ms; zero with fewer than two samples in the window.
    pub fn get(&self) -> (f32, f32) {
        if self.samples.len() < 2 {
            return (0.0, 0.0);
        }
        let last = self.samples[self.samples.len() - 1].0;
        let ts: Vec<f64> = self.samples.iter().map(|s| -((last - s.0).as_secs_f64() * 1000.0)).collect();
        let xs: Vec<f64> = self.samples.iter().map(|s| s.1 as f64).collect();
        let ys: Vec<f64> = self.samples.iter().map(|s| s.2 as f64).collect();
        (slope(&ts, &xs) as f32, slope(&ts, &ys) as f32)
    }
}

/// Slope at t = 0 (the newest sample) of the best-fitting curve
/// a + b t + c t^2, or of a straight line when there are only two points.
fn slope(ts: &[f64], vs: &[f64]) -> f64 {
    let line = || {
        let (t0, v0, t1, v1) = (ts[0], vs[0], ts[ts.len() - 1], vs[vs.len() - 1]);
        if t1 - t0 > 0.0 { (v1 - v0) / (t1 - t0) } else { 0.0 }
    };
    if ts.len() < 3 {
        return line();
    }
    // Sums for the normal equations of a least-squares fit.
    let mut st = [0.0f64; 5];
    let mut sv = [0.0f64; 3];
    for (&t, &v) in ts.iter().zip(vs) {
        let mut p = 1.0;
        for (k, s) in st.iter_mut().enumerate() {
            *s += p;
            if k < 3 {
                sv[k] += p * v;
            }
            p *= t;
        }
    }
    let m = [[st[0], st[1], st[2]], [st[1], st[2], st[3]], [st[2], st[3], st[4]]];
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&m);
    if d.abs() < 1e-9 {
        return line();
    }
    // Cramer's rule for b, the middle unknown.
    let mut mb = m;
    for r in 0..3 {
        mb[r][1] = sv[r];
    }
    det(&mb) / d
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
        assert!((vy - -2.0).abs() < 0.01, "{vy}");
    }

    #[test]
    fn speeding_up_flick_reads_the_speed_at_release() {
        // y = 800 - 0.01 t^2: the finger is moving 1.92 px/ms when it lifts
        // at 96 ms, though its average over the window is only half that.
        let t0 = Instant::now();
        let mut v = Velocity::new();
        for n in (0..=96u64).step_by(8) {
            let t = n as f32;
            v.add(t0 + Duration::from_millis(n), 0.0, 800.0 - 0.01 * t * t);
        }
        let (_, vy) = v.get();
        assert!((vy - -1.92).abs() < 0.05, "{vy}");
    }

    #[test]
    fn lifting_keeps_the_speed_of_the_last_moves() {
        let t0 = Instant::now();
        let mut v = Velocity::new();
        for n in 0..6u64 {
            v.add(t0 + Duration::from_millis(8 * n), 0.0, 900.0 - 50.0 * n as f32);
        }
        v.lift(t0 + Duration::from_millis(45));
        let (_, vy) = v.get();
        assert!((vy - -6.25).abs() < 0.01, "{vy}");
        v.lift(t0 + Duration::from_millis(90));
        assert_eq!(v.get(), (0.0, 0.0));
    }

    #[test]
    fn pausing_before_lifting_is_still() {
        let t0 = Instant::now();
        let mut v = Velocity::new();
        v.add(t0, 0.0, 800.0);
        v.add(t0 + Duration::from_millis(16), 0.0, 760.0);
        v.add(t0 + Duration::from_millis(32), 0.0, 720.0);
        v.add(t0 + Duration::from_millis(90), 0.0, 720.0);
        assert_eq!(v.get(), (0.0, 0.0));
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
