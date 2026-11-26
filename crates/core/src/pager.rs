//! Sideways paging of the home screen: follows the finger, then snaps.

use std::time::{Duration, Instant};

use crate::velocity::Velocity;

pub const SNAP: Duration = Duration::from_millis(220);
const FLING_PX_PER_MS: f32 = 0.5;
const DISTANCE: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    Tap { x: f32, y: f32 },
    PageSwipe,
    VerticalSwipe { dy: f32 },
    None,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lock {
    Undecided,
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    start_x: f32,
    start_y: f32,
    start_offset: f32,
    start_page: usize,
    x: f32,
    y: f32,
    lock: Lock,
}

#[derive(Clone, Copy, Debug)]
struct Snap {
    from: f32,
    start: Instant,
}

#[derive(Debug)]
pub struct Pager {
    pages: usize,
    page_w: f32,
    slop: f32,
    page: usize,
    drag: Option<Drag>,
    snap: Option<Snap>,
    velocity: Velocity,
    snap_time: Duration,
}

impl Pager {
    pub fn new(pages: usize, page_w: f32) -> Pager {
        Pager::new_with_slop(pages, page_w, 24.0)
    }

    pub fn new_with_slop(pages: usize, page_w: f32, slop_px: f32) -> Pager {
        Pager { pages: pages.max(1), page_w, slop: slop_px, page: 0, drag: None, snap: None, velocity: Velocity::new(), snap_time: SNAP }
    }

    pub fn set_pages(&mut self, n: usize) {
        self.pages = n.max(1);
        if self.page >= self.pages {
            self.page = self.pages - 1;
            self.snap = None;
        }
    }

    /// How long a slide to a page takes (zero: it jumps).
    pub fn set_snap(&mut self, d: Duration) {
        self.snap_time = d;
    }

    pub fn set_width(&mut self, w: f32) {
        self.page_w = w;
        self.snap = None;
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn pages(&self) -> usize {
        self.pages
    }

    /// True once the finger has moved far enough to stop being a tap.
    pub fn moved(&self) -> bool {
        self.drag.is_some_and(|d| d.lock != Lock::Undecided)
    }

    /// True while a finger is down and has turned into a sideways drag.
    pub fn dragging_pages(&self) -> bool {
        self.drag.is_some_and(|d| d.lock == Lock::Horizontal)
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.snap.is_some_and(|s| now < s.start + self.snap_time)
    }

    fn max_offset(&self) -> f32 {
        (self.pages - 1) as f32 * self.page_w
    }

    pub fn offset(&self, now: Instant) -> f32 {
        let rest = self.page as f32 * self.page_w;
        if let Some(d) = self.drag {
            if d.lock != Lock::Horizontal {
                return d.start_offset;
            }
            let raw = d.start_offset - (d.x - d.start_x);
            let max = self.max_offset();
            return if raw < 0.0 {
                raw / 3.0
            } else if raw > max {
                max + (raw - max) / 3.0
            } else {
                raw
            };
        }
        match self.snap {
            Some(s) if now < s.start + self.snap_time => {
                let t = (now - s.start).as_secs_f32() / self.snap_time.as_secs_f32();
                let e = 1.0 - (1.0 - t).powi(3);
                s.from + (rest - s.from) * e
            }
            _ => rest,
        }
    }

    pub fn touch_down(&mut self, x: f32, y: f32, t: Instant) {
        // Catch the page where it is if it's still sliding.
        let start_offset = self.offset(t);
        self.snap = None;
        let start_page = ((start_offset / self.page_w).round().max(0.0) as usize).min(self.pages - 1);
        self.page = start_page;
        self.drag = Some(Drag { start_x: x, start_y: y, start_offset, start_page, x, y, lock: Lock::Undecided });
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
        d.x = x;
        d.y = y;
        if d.lock == Lock::Undecided {
            let (dx, dy) = ((x - d.start_x).abs(), (y - d.start_y).abs());
            if dx > slop && dx > dy {
                d.lock = Lock::Horizontal;
            } else if dy > slop && dy >= dx {
                d.lock = Lock::Vertical;
            }
        }
    }

    pub fn touch_up(&mut self, x: f32, y: f32, t: Instant) -> Gesture {
        self.follow(x, y);
        self.velocity.lift(t);
        let Some(d) = self.drag else { return Gesture::None };
        match d.lock {
            Lock::Undecided => {
                self.drag = None;
                Gesture::Tap { x, y }
            }
            Lock::Vertical => {
                self.drag = None;
                Gesture::VerticalSwipe { dy: y - d.start_y }
            }
            Lock::Horizontal => {
                let from = self.offset(t);
                self.drag = None;
                let dragged = from - d.start_page as f32 * self.page_w;
                let speed = -self.velocity.get().0; // finger left = offset grows
                let target = if speed > FLING_PX_PER_MS || (speed > -FLING_PX_PER_MS && dragged > DISTANCE * self.page_w) {
                    d.start_page + 1
                } else if speed < -FLING_PX_PER_MS || dragged < -DISTANCE * self.page_w {
                    d.start_page.saturating_sub(1)
                } else {
                    d.start_page
                };
                self.page = target.min(self.pages - 1);
                self.snap = Some(Snap { from, start: t });
                Gesture::PageSwipe
            }
        }
    }

    /// Be on `page` (clamped) at once, with no slide.
    pub fn jump_to(&mut self, page: usize) {
        self.drag = None;
        self.snap = None;
        self.page = page.min(self.pages - 1);
    }

    /// Slide to `page` (clamped), dropping any drag in progress.
    pub fn snap_to(&mut self, page: usize, t: Instant) {
        let from = self.offset(t);
        self.drag = None;
        self.page = page.min(self.pages - 1);
        self.snap = Some(Snap { from, start: t });
    }

    pub fn touch_cancel(&mut self, t: Instant) {
        if let Some(d) = self.drag.take() {
            if d.lock == Lock::Horizontal {
                let from = {
                    self.drag = Some(d);
                    let o = self.offset(t);
                    self.drag = None;
                    o
                };
                self.page = d.start_page;
                self.snap = Some(Snap { from, start: t });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 1080.0;

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn jump_to_lands_without_sliding() {
        let t0 = Instant::now();
        let mut p = Pager::new(10, W);
        p.jump_to(7);
        assert_eq!(p.page(), 7);
        assert_eq!(p.offset(t0), 7.0 * W);
        assert!(!p.animating(t0));
        p.jump_to(99);
        assert_eq!(p.page(), 9);
    }

    #[test]
    fn small_move_is_a_tap() {
        let t0 = Instant::now();
        let mut p = Pager::new(2, W);
        p.touch_down(500.0, 500.0, t0);
        p.touch_move(503.0, 504.0, ms(t0, 30));
        assert_eq!(p.touch_up(505.0, 500.0, ms(t0, 60)), Gesture::Tap { x: 505.0, y: 500.0 });
        assert_eq!(p.page(), 0);
        assert_eq!(p.offset(ms(t0, 60)), 0.0);
    }

    #[test]
    fn long_slow_drag_turns_the_page() {
        let t0 = Instant::now();
        let mut p = Pager::new(2, W);
        p.touch_down(800.0, 500.0, t0);
        p.touch_move(368.0, 510.0, ms(t0, 300));
        assert!(p.dragging_pages());
        assert!((p.offset(ms(t0, 300)) - 432.0).abs() < 0.01);
        assert_eq!(p.touch_up(368.0, 510.0, ms(t0, 600)), Gesture::PageSwipe);
        assert_eq!(p.page(), 1);
        assert!(p.animating(ms(t0, 700)));
        assert!(!p.animating(ms(t0, 820)));
        assert_eq!(p.offset(ms(t0, 820)), W);
    }

    #[test]
    fn short_slow_drag_springs_back() {
        let t0 = Instant::now();
        let mut p = Pager::new(2, W);
        p.touch_down(800.0, 500.0, t0);
        p.touch_move(692.0, 500.0, ms(t0, 500));
        assert_eq!(p.touch_up(692.0, 500.0, ms(t0, 1000)), Gesture::PageSwipe);
        assert_eq!(p.page(), 0);
        assert_eq!(p.offset(ms(t0, 1300)), 0.0);
    }

    #[test]
    fn short_fast_flick_turns_the_page() {
        let t0 = Instant::now();
        let mut p = Pager::new(2, W);
        p.touch_down(800.0, 500.0, t0);
        p.touch_move(760.0, 500.0, ms(t0, 25));
        assert_eq!(p.touch_up(720.0, 500.0, ms(t0, 50)), Gesture::PageSwipe);
        assert_eq!(p.page(), 1);
    }

    #[test]
    fn flick_back_from_page_one() {
        let t0 = Instant::now();
        let mut p = Pager::new(2, W);
        p.touch_down(800.0, 500.0, t0);
        p.touch_move(300.0, 500.0, ms(t0, 100));
        p.touch_up(300.0, 500.0, ms(t0, 400));
        assert_eq!(p.page(), 1);
        let t1 = ms(t0, 1000);
        p.touch_down(300.0, 500.0, t1);
        p.touch_move(340.0, 500.0, ms(t1, 25));
        p.touch_up(380.0, 500.0, ms(t1, 50));
        assert_eq!(p.page(), 0);
    }

    #[test]
    fn lift_repeating_the_last_point_still_flicks() {
        let t0 = Instant::now();
        let mut p = Pager::new(2, W);
        p.touch_down(800.0, 500.0, t0);
        p.touch_move(760.0, 500.0, ms(t0, 20));
        p.touch_move(720.0, 500.0, ms(t0, 40));
        p.touch_up(720.0, 500.0, ms(t0, 48));
        assert_eq!(p.page(), 1);
    }

    #[test]
    fn zero_snap_time_jumps() {
        let t0 = Instant::now();
        let mut p = Pager::new(3, W);
        p.set_snap(Duration::ZERO);
        p.snap_to(2, t0);
        assert!(!p.animating(t0));
        assert_eq!(p.offset(t0), 2.0 * W);
        p.set_snap(Duration::from_millis(400));
        p.snap_to(0, t0);
        assert!(p.animating(ms(t0, 300)));
    }

    #[test]
    fn snap_to_slides_to_a_page() {
        let t0 = Instant::now();
        let mut p = Pager::new(3, W);
        p.touch_down(800.0, 500.0, t0);
        p.touch_move(100.0, 500.0, ms(t0, 100));
        p.touch_up(100.0, 500.0, ms(t0, 400));
        assert_eq!(p.page(), 1);
        p.snap_to(0, ms(t0, 1000));
        assert_eq!(p.page(), 0);
        assert!(p.animating(ms(t0, 1100)));
        assert_eq!(p.offset(ms(t0, 1300)), 0.0);
        p.snap_to(9, ms(t0, 2000));
        assert_eq!(p.page(), 2);
    }

    #[test]
    fn last_page_does_not_move_on() {
        let t0 = Instant::now();
        let mut p = Pager::new(1, W);
        p.touch_down(800.0, 500.0, t0);
        p.touch_move(200.0, 500.0, ms(t0, 100));
        p.touch_up(100.0, 500.0, ms(t0, 120));
        assert_eq!(p.page(), 0);
        assert_eq!(p.offset(ms(t0, 400)), 0.0);
    }

    #[test]
    fn mostly_vertical_swipe_keeps_the_page() {
        let t0 = Instant::now();
        let mut p = Pager::new(2, W);
        p.touch_down(500.0, 500.0, t0);
        p.touch_move(505.0, 510.0, ms(t0, 50));
        assert!(!p.moved());
        p.touch_move(510.0, 700.0, ms(t0, 60));
        p.touch_move(520.0, 800.0, ms(t0, 100));
        assert!(p.moved());
        assert!(!p.dragging_pages());
        match p.touch_up(520.0, 800.0, ms(t0, 120)) {
            Gesture::VerticalSwipe { dy } => assert!((dy - 300.0).abs() < 0.01),
            g => panic!("got {g:?}"),
        }
        assert_eq!(p.page(), 0);
        assert_eq!(p.offset(ms(t0, 120)), 0.0);
    }

    #[test]
    fn dragging_past_the_first_page_resists() {
        let t0 = Instant::now();
        let mut p = Pager::new(2, W);
        p.touch_down(200.0, 500.0, t0);
        p.touch_move(500.0, 500.0, ms(t0, 100));
        assert!((p.offset(ms(t0, 100)) + 100.0).abs() < 0.01);
    }

    #[test]
    fn fewer_pages_pulls_the_page_back() {
        let t0 = Instant::now();
        let mut p = Pager::new(3, W);
        p.touch_down(800.0, 500.0, t0);
        p.touch_move(100.0, 500.0, ms(t0, 100));
        p.touch_up(100.0, 500.0, ms(t0, 400));
        p.touch_down(800.0, 500.0, ms(t0, 1000));
        p.touch_move(100.0, 500.0, ms(t0, 1100));
        p.touch_up(100.0, 500.0, ms(t0, 1400));
        assert_eq!(p.page(), 2);
        p.set_pages(1);
        assert_eq!(p.page(), 0);
        assert_eq!(p.offset(ms(t0, 1500)), 0.0);
    }
}
