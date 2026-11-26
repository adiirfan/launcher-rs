//! Up-and-down scrolling of a list, with a flick that slows to a stop.

use std::time::Instant;
#[cfg(test)]
use std::time::Duration;

use crate::velocity::Velocity;

/// The glide after a flick matches Samsung's own lists, measured on the
/// Fold with identical swipes: it runs GLIDE_DP * speed^GLIDE_POWER dp for a
/// release at `speed` dp per ms, along Android's usual slowing-down curve.
/// That curve's average speed is INFLEXION times the release speed.
const GLIDE_DP: f32 = 345.0;
const GLIDE_POWER: f32 = 1.3;
const INFLEXION: f32 = 0.35;
/// Slowest and fastest flick, in dp per second.
const FLING_MIN_DP: f32 = 50.0;
const FLING_MAX_DP: f32 = 8000.0;
const SAMPLES: usize = 100;

/// Share of the glide covered at each 1% of its time.
fn spline() -> &'static [f32; SAMPLES + 1] {
    static TABLE: std::sync::OnceLock<[f32; SAMPLES + 1]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let (p1, p2) = (0.5 * INFLEXION, 1.0 - (1.0 - INFLEXION));
        let mut table = [1.0f32; SAMPLES + 1];
        let mut x_min = 0.0f32;
        for (i, slot) in table.iter_mut().take(SAMPLES).enumerate() {
            let alpha = i as f32 / SAMPLES as f32;
            let mut x_max = 1.0f32;
            let (mut x, mut coef);
            loop {
                x = x_min + (x_max - x_min) / 2.0;
                coef = 3.0 * x * (1.0 - x);
                let tx = coef * ((1.0 - x) * p1 + x * p2) + x * x * x;
                if (tx - alpha).abs() < 1e-5 {
                    break;
                }
                if tx > alpha { x_max = x } else { x_min = x }
            }
            *slot = coef * ((1.0 - x) * 0.5 + x) + x * x * x;
        }
        table[0] = 0.0;
        table
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ListGesture {
    Tap { x: f32, y: f32 },
    Scroll,
    PullDown { dy: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lock {
    Undecided,
    Scroll,
    Pull,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    start_x: f32,
    start_y: f32,
    start_pos: f32,
    y: f32,
    lock: Lock,
    /// The touch stopped a flick, so lifting it isn't a tap.
    caught: bool,
}

#[derive(Clone, Copy, Debug)]
struct Fling {
    from: f32,
    /// +1 towards the end of the list, -1 towards the top.
    dir: f32,
    distance: f32,
    duration_ms: f32,
    start: Instant,
}

#[derive(Debug)]
pub struct Scroller {
    slop: f32,
    density: f32,
    max: f32,
    pos: f32,
    drag: Option<Drag>,
    fling: Option<Fling>,
    velocity: Velocity,
}

impl Scroller {
    pub fn new(slop_px: f32, density: f32) -> Scroller {
        Scroller { slop: slop_px, density, max: 0.0, pos: 0.0, drag: None, fling: None, velocity: Velocity::new() }
    }

    pub fn set_bounds(&mut self, max: f32) {
        self.max = max.max(0.0);
        self.pos = self.pos.clamp(0.0, self.max);
    }

    /// Show `pos` at once (kept in bounds), stopping any fling or drag.
    pub fn jump_to(&mut self, pos: f32) {
        self.pos = self.clamp(pos);
        self.drag = None;
        self.fling = None;
    }

    pub fn reset(&mut self) {
        self.pos = 0.0;
        self.drag = None;
        self.fling = None;
    }

    pub fn moved(&self) -> bool {
        self.drag.is_some_and(|d| d.lock != Lock::Undecided)
    }

    fn clamp(&self, v: f32) -> f32 {
        v.clamp(0.0, self.max)
    }

    /// The glide for a release at `v` px per ms, or none if too slow.
    fn fling(&self, from: f32, v: f32, start: Instant) -> Option<Fling> {
        let dp_s = v.abs() * 1000.0 / self.density;
        if dp_s < FLING_MIN_DP {
            return None;
        }
        let dp_ms = dp_s.min(FLING_MAX_DP) / 1000.0;
        let distance = GLIDE_DP * dp_ms.powf(GLIDE_POWER) * self.density;
        let duration_ms = distance / (INFLEXION * dp_ms * self.density);
        Some(Fling { from, dir: v.signum(), distance, duration_ms, start })
    }

    /// Where the glide is at `now`, and whether it is still going.
    fn fling_at(&self, f: &Fling, now: Instant) -> (f32, bool) {
        let t = now.saturating_duration_since(f.start).as_secs_f32() * 1000.0;
        let frac = (t / f.duration_ms).min(1.0);
        let table = spline();
        let i = ((SAMPLES as f32 * frac) as usize).min(SAMPLES - 1);
        let (t_inf, t_sup) = (i as f32 / SAMPLES as f32, (i + 1) as f32 / SAMPLES as f32);
        let share = table[i] + (frac - t_inf) / (t_sup - t_inf) * (table[i + 1] - table[i]);
        (self.clamp(f.from + f.dir * f.distance * share), frac < 1.0)
    }

    pub fn offset(&self, now: Instant) -> f32 {
        if let Some(d) = self.drag {
            return match d.lock {
                Lock::Scroll => self.clamp(d.start_pos - (d.y - d.start_y)),
                _ => d.start_pos,
            };
        }
        match &self.fling {
            Some(f) => self.fling_at(f, now).0,
            None => self.pos,
        }
    }

    pub fn animating(&self, now: Instant) -> bool {
        let Some(f) = &self.fling else { return false };
        if self.drag.is_some() {
            return false;
        }
        let (pos, going) = self.fling_at(f, now);
        let at_end = (f.dir > 0.0 && pos >= self.max) || (f.dir < 0.0 && pos <= 0.0);
        going && !at_end
    }

    pub fn touch_down(&mut self, x: f32, y: f32, t: Instant) {
        let caught = self.animating(t);
        self.pos = self.offset(t);
        self.fling = None;
        self.drag = Some(Drag { start_x: x, start_y: y, start_pos: self.pos, y, lock: Lock::Undecided, caught });
        self.velocity.clear();
        self.velocity.add(t, x, y);
    }

    pub fn touch_move(&mut self, x: f32, y: f32, t: Instant) {
        if self.drag.is_some() {
            self.velocity.add(t, x, y);
        }
        self.follow(x, y);
    }

    fn follow(&mut self, x: f32, y: f32) {
        let slop = self.slop;
        let Some(d) = self.drag.as_mut() else { return };
        d.y = y;
        if d.lock == Lock::Undecided {
            let (dx, dy) = (x - d.start_x, y - d.start_y);
            if dx.abs() > slop || dy.abs() > slop {
                d.lock = if dy > 0.0 && dy.abs() >= dx.abs() && d.start_pos <= 0.5 { Lock::Pull } else { Lock::Scroll };
            }
        }
    }

    pub fn touch_up(&mut self, x: f32, y: f32, t: Instant) -> ListGesture {
        self.follow(x, y);
        self.velocity.lift(t);
        let Some(d) = self.drag else { return ListGesture::Scroll };
        let pos = self.offset(t);
        self.drag = None;
        self.pos = pos;
        match d.lock {
            Lock::Undecided if d.caught => ListGesture::Scroll,
            Lock::Undecided => ListGesture::Tap { x, y },
            Lock::Pull => ListGesture::PullDown { dy: y - d.start_y },
            Lock::Scroll => {
                let v = -self.velocity.get().1;
                self.fling = self.fling(pos, v, t);
                ListGesture::Scroll
            }
        }
    }

    /// The current glide's length in px and time in ms, for logging.
    pub fn glide(&self) -> Option<(f32, f32)> {
        self.fling.as_ref().map(|f| (f.distance, f.duration_ms))
    }

    pub fn touch_cancel(&mut self, t: Instant) {
        self.pos = self.offset(t);
        self.drag = None;
    }

    /// How far past an end the finger is pushing right now: negative while
    /// pulling down at the top, positive past the bottom, else 0.
    pub fn overscroll(&self) -> f32 {
        let Some(d) = self.drag else { return 0.0 };
        match d.lock {
            Lock::Pull => -(d.y - d.start_y).max(0.0),
            Lock::Scroll => {
                let raw = d.start_pos - (d.y - d.start_y);
                if raw > self.max {
                    raw - self.max
                } else {
                    raw.min(0.0)
                }
            }
            Lock::Undecided => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    fn list() -> Scroller {
        let mut s = Scroller::new(24.0, 3.0);
        s.set_bounds(1000.0);
        s
    }

    #[test]
    fn jump_to_moves_and_stops_a_fling() {
        let t0 = Instant::now();
        let mut s = Scroller::new(24.0, 3.0);
        s.set_bounds(5000.0);
        s.touch_down(500.0, 1500.0, t0);
        for k in 1..=10 {
            s.touch_move(500.0, 1500.0 - 60.0 * k as f32, t0 + Duration::from_millis(8 * k));
        }
        s.touch_up(500.0, 900.0, t0 + Duration::from_millis(88));
        assert!(s.animating(t0 + Duration::from_millis(100)));
        s.jump_to(1234.0);
        assert!(!s.animating(t0 + Duration::from_millis(100)));
        assert_eq!(s.offset(t0 + Duration::from_millis(100)), 1234.0);
        s.jump_to(9999.0);
        assert_eq!(s.offset(t0), 5000.0);
    }

    #[test]
    fn overscroll_reads_the_push_past_either_end() {
        let t0 = Instant::now();
        let mut s = list();
        assert_eq!(s.overscroll(), 0.0);
        s.touch_down(100.0, 500.0, t0);
        s.touch_move(100.0, 700.0, ms(t0, 100));
        assert_eq!(s.overscroll(), -200.0);
        s.touch_up(100.0, 700.0, ms(t0, 200));
        assert_eq!(s.overscroll(), 0.0);
        s.touch_down(100.0, 1500.0, ms(t0, 300));
        s.touch_move(100.0, 300.0, ms(t0, 400));
        assert_eq!(s.overscroll(), 200.0);
        s.touch_move(100.0, 800.0, ms(t0, 500));
        assert_eq!(s.overscroll(), 0.0);
    }

    #[test]
    fn small_move_is_a_tap() {
        let t0 = Instant::now();
        let mut s = list();
        s.touch_down(100.0, 1000.0, t0);
        s.touch_move(104.0, 1006.0, ms(t0, 40));
        assert!(!s.moved());
        assert_eq!(s.touch_up(104.0, 1006.0, ms(t0, 80)), ListGesture::Tap { x: 104.0, y: 1006.0 });
        assert_eq!(s.offset(ms(t0, 80)), 0.0);
    }

    #[test]
    fn drag_up_scrolls_and_stops_at_the_ends() {
        let t0 = Instant::now();
        let mut s = list();
        s.touch_down(100.0, 1500.0, t0);
        s.touch_move(100.0, 1200.0, ms(t0, 300));
        assert!(s.moved());
        assert_eq!(s.offset(ms(t0, 300)), 300.0);
        s.touch_move(100.0, 100.0, ms(t0, 600));
        assert_eq!(s.offset(ms(t0, 600)), 1000.0);
        assert_eq!(s.touch_up(100.0, 100.0, ms(t0, 900)), ListGesture::Scroll);
        assert_eq!(s.offset(ms(t0, 2000)), 1000.0);
        assert!(!s.animating(ms(t0, 900)));
    }

    #[test]
    fn flick_keeps_going_then_stops() {
        let t0 = Instant::now();
        let mut s = list();
        s.touch_down(100.0, 1500.0, t0);
        s.touch_move(100.0, 1450.0, ms(t0, 30));
        s.touch_move(100.0, 1400.0, ms(t0, 60));
        s.touch_up(100.0, 1400.0, ms(t0, 64));
        assert_eq!(s.offset(ms(t0, 64)), 100.0);
        assert!(s.animating(ms(t0, 100)));
        let later = s.offset(ms(t0, 200));
        assert!(later > 150.0 && later < 1000.0, "{later}");
        let mut last = 0.0;
        for n in (64..3000).step_by(16) {
            let o = s.offset(ms(t0, n));
            assert!(o >= last - 0.001 && o <= 1000.0);
            last = o;
        }
        assert!(!s.animating(ms(t0, 3000)));
    }

    #[test]
    fn long_flick_stops_at_the_end() {
        let t0 = Instant::now();
        let mut s = list();
        s.touch_down(100.0, 1500.0, t0);
        s.touch_move(100.0, 1300.0, ms(t0, 30));
        s.touch_move(100.0, 1100.0, ms(t0, 60));
        s.touch_up(100.0, 1100.0, ms(t0, 64));
        assert_eq!(s.offset(ms(t0, 2000)), 1000.0);
        assert!(!s.animating(ms(t0, 2000)));
    }

    /// A steady flick up at `speed` px/ms on a very long list; returns
    /// where the finger left it and where the glide came to rest.
    fn steady_flick(speed: f32) -> (Scroller, Instant, f32) {
        let t0 = Instant::now();
        let mut s = Scroller::new(24.0, 3.0);
        s.set_bounds(100_000.0);
        s.touch_down(100.0, 1500.0, t0);
        for n in 1..=8u64 {
            s.touch_move(100.0, 1500.0 - speed * 8.0 * n as f32, ms(t0, 8 * n));
        }
        let y = 1500.0 - speed * 64.0;
        s.touch_up(100.0, y, ms(t0, 66));
        (s, t0, 1500.0 - y)
    }

    fn glide(speed: f32) -> f32 {
        let (s, t0, released) = steady_flick(speed);
        s.offset(ms(t0, 10_000)) - released
    }

    fn near(got: f32, want: f32) -> bool {
        (got - want).abs() <= want * 0.01
    }

    #[test]
    fn glide_goes_as_far_as_samsung_lists() {
        // Samsung's own lists at 480 dpi glide this far for 6000 px/s.
        let g = glide(6.0);
        assert!(near(g, 2548.5), "{g}");
        let (s, t0, _) = steady_flick(6.0);
        let (d, ms_) = s.glide().unwrap();
        assert!(near(d, 2548.5) && near(ms_, 1213.6), "{d} {ms_}");
        assert!(s.animating(ms(t0, 66 + 1190)));
        assert!(!s.animating(ms(t0, 66 + 1240)));
    }

    #[test]
    fn faster_flicks_glide_much_further() {
        let g = glide(12.0);
        assert!(near(g, 6275.1), "{g}");
    }

    #[test]
    fn glide_speed_has_a_ceiling() {
        // 8000 dp/s is Android's top fling speed: 24 px/ms at 3x.
        let g = glide(40.0);
        assert!(near(g, 15451.0), "{g}");
    }

    #[test]
    fn glide_never_goes_backwards_and_starts_at_finger_speed() {
        let (s, t0, released) = steady_flick(6.0);
        let first = s.offset(ms(t0, 66 + 16)) - released;
        assert!(first > 6.0 * 16.0 * 0.8 && first < 6.0 * 16.0 * 1.2, "{first}");
        let mut last = released;
        for n in (66..2000).step_by(8) {
            let o = s.offset(ms(t0, n));
            assert!(o >= last - 0.001, "{n}: {o} < {last}");
            last = o;
        }
    }

    #[test]
    fn touching_a_flick_catches_it() {
        let t0 = Instant::now();
        let mut s = list();
        s.touch_down(100.0, 1500.0, t0);
        s.touch_move(100.0, 1450.0, ms(t0, 30));
        s.touch_move(100.0, 1400.0, ms(t0, 60));
        s.touch_up(100.0, 1400.0, ms(t0, 64));
        let caught = s.offset(ms(t0, 150));
        s.touch_down(100.0, 900.0, ms(t0, 150));
        assert!(!s.animating(ms(t0, 200)));
        assert_eq!(s.offset(ms(t0, 400)), caught);
        assert_eq!(s.touch_up(100.0, 900.0, ms(t0, 200)), ListGesture::Scroll);
        assert_eq!(s.offset(ms(t0, 400)), caught);
    }

    #[test]
    fn pulling_down_at_the_top_is_its_own_gesture() {
        let t0 = Instant::now();
        let mut s = list();
        s.touch_down(100.0, 500.0, t0);
        s.touch_move(100.0, 700.0, ms(t0, 100));
        assert_eq!(s.offset(ms(t0, 100)), 0.0);
        match s.touch_up(100.0, 700.0, ms(t0, 150)) {
            ListGesture::PullDown { dy } => assert!((dy - 200.0).abs() < 0.01),
            g => panic!("{g:?}"),
        }
    }

    #[test]
    fn pulling_down_mid_list_scrolls_back() {
        let t0 = Instant::now();
        let mut s = list();
        s.touch_down(100.0, 900.0, t0);
        s.touch_move(100.0, 800.0, ms(t0, 300));
        s.touch_up(100.0, 800.0, ms(t0, 600));
        s.touch_down(100.0, 500.0, ms(t0, 1000));
        s.touch_move(100.0, 560.0, ms(t0, 1300));
        assert_eq!(s.offset(ms(t0, 1300)), 40.0);
        assert_eq!(s.touch_up(100.0, 560.0, ms(t0, 1600)), ListGesture::Scroll);
    }

    #[test]
    fn shrinking_the_list_clamps_and_reset_returns_to_top() {
        let t0 = Instant::now();
        let mut s = list();
        s.touch_down(100.0, 1500.0, t0);
        s.touch_move(100.0, 800.0, ms(t0, 300));
        s.touch_up(100.0, 800.0, ms(t0, 600));
        s.set_bounds(200.0);
        assert_eq!(s.offset(ms(t0, 700)), 200.0);
        s.reset();
        assert_eq!(s.offset(ms(t0, 700)), 0.0);
    }
}
