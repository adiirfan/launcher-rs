//! Up-and-down scrolling of a list, with a flick that slows to a stop.

use std::time::Instant;
#[cfg(test)]
use std::time::Duration;

use crate::velocity::Velocity;

/// How quickly a flick slows down: it covers 63% of its run in this time.
const TAU_MS: f32 = 325.0;
const FLING_MIN: f32 = 0.3;

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
    /// Offset speed, px per ms (positive = towards the end of the list).
    v: f32,
    start: Instant,
}

#[derive(Debug)]
pub struct Scroller {
    slop: f32,
    max: f32,
    pos: f32,
    drag: Option<Drag>,
    fling: Option<Fling>,
    velocity: Velocity,
}

impl Scroller {
    pub fn new(slop_px: f32) -> Scroller {
        Scroller { slop: slop_px, max: 0.0, pos: 0.0, drag: None, fling: None, velocity: Velocity::new() }
    }

    pub fn set_bounds(&mut self, max: f32) {
        self.max = max.max(0.0);
        self.pos = self.pos.clamp(0.0, self.max);
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

    fn fling_at(&self, f: &Fling, now: Instant) -> (f32, f32) {
        let t = now.saturating_duration_since(f.start).as_secs_f32() * 1000.0;
        let decay = (-t / TAU_MS).exp();
        let pos = f.from + f.v * TAU_MS * (1.0 - decay);
        (self.clamp(pos), f.v.abs() * TAU_MS * decay)
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
        let (pos, left) = self.fling_at(f, now);
        let at_end = (f.v > 0.0 && pos >= self.max) || (f.v < 0.0 && pos <= 0.0);
        left > 0.5 && !at_end
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
        let slop = self.slop;
        let Some(d) = self.drag.as_mut() else { return };
        self.velocity.add(t, x, y);
        d.y = y;
        if d.lock == Lock::Undecided {
            let (dx, dy) = (x - d.start_x, y - d.start_y);
            if dx.abs() > slop || dy.abs() > slop {
                d.lock = if dy > 0.0 && dy.abs() >= dx.abs() && d.start_pos <= 0.5 { Lock::Pull } else { Lock::Scroll };
            }
        }
    }

    pub fn touch_up(&mut self, x: f32, y: f32, t: Instant) -> ListGesture {
        self.touch_move(x, y, t);
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
                if v.abs() > FLING_MIN {
                    self.fling = Some(Fling { from: pos, v, start: t });
                }
                ListGesture::Scroll
            }
        }
    }

    pub fn touch_cancel(&mut self, t: Instant) {
        self.pos = self.offset(t);
        self.drag = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    fn list() -> Scroller {
        let mut s = Scroller::new(24.0);
        s.set_bounds(1000.0);
        s
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
        s.touch_move(100.0, 1400.0, ms(t0, 30));
        s.touch_move(100.0, 1300.0, ms(t0, 60));
        s.touch_up(100.0, 1300.0, ms(t0, 64));
        assert_eq!(s.offset(ms(t0, 2000)), 1000.0);
        assert!(!s.animating(ms(t0, 2000)));
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
