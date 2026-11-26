//! Make a GIF from a video, as the approved mockup: a preview with the GIF
//! badge; PART OF THE VIDEO (tap to choose it on the clip strip); frames a
//! second; width; PLAYS; the size note (a warning over 10 MB); Cancel and
//! Make GIF.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use android_activity::AndroidApp;
use gallery_core::clip::{time_label, ClipRange};
use gallery_core::format;
use gallery_core::gif_plan::{GifPlan, Plays, TOO_BIG};
use gallery_core::media::MediaItem;
use glow::Context;
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, WHITE};
use holo_platform::render::Renderer;
use launcher_core::scroll::{ListGesture, Scroller};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::gif_maker::{self, Making};
use crate::input::{Touch, TouchKind};
use crate::photos::Frame;
use crate::{thumbs, widgets};

const BAR_DP: f32 = 56.0;
const BOTTOM_DP: f32 = 56.0;
const PREVIEW_DP: f32 = 200.0;
const PLAY_ROW_DP: f32 = 56.0;
const QUIET: u32 = 0xBDBDBD;
const SUB: u32 = 0x9E9E9E;
const WARN: u32 = 0xFFBB33;
const LINE: u32 = 0x262626;
/// The part chosen at first: up to this long.
const DEFAULT_MS: u64 = 6000;

const FPS: [u32; 4] = [10, 15, 20, 30];
const WIDTHS: [(u32, &str); 4] = [(320, "320"), (480, "480"), (640, "640"), (0, "FULL")];
const PLAYS: [(Plays, &str); 3] = [(Plays::Loop, "Loop forever"), (Plays::Bounce, "Forwards then backwards"), (Plays::Backwards, "Backwards")];

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Close,
    /// Open the clip strip to choose the part, starting from this one.
    ChoosePart(ClipRange),
    /// Made: the new GIF's name.
    Done(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Back,
    Part,
    Fps(usize),
    Width(usize),
    Plays(usize),
    Cancel,
    Make,
}

struct Lay {
    preview_y: f32,
    part_y: f32,
    fps_y: f32,
    width_y: f32,
    plays_y: f32,
    note_y: f32,
    height: f32,
}

pub struct GifScreen {
    app: AndroidApp,
    d: f32,
    item: MediaItem,
    range: ClipRange,
    fps: u32,
    width: u32,
    plays: Plays,
    scroller: Scroller,
    press: Option<Hit>,
    last: Option<Frame>,
    making: Option<Arc<Making>>,
    error: Option<String>,
}

impl GifScreen {
    /// `at_ms`: where the video was; the part starts there.
    pub fn new(app: &AndroidApp, d: f32, item: MediaItem, at_ms: u64) -> GifScreen {
        let dur = item.duration_ms;
        let mut range = ClipRange::new(dur);
        let start = if at_ms + 500 >= dur { dur.saturating_sub(DEFAULT_MS) } else { at_ms };
        range.set_start(start);
        range.set_end(start + DEFAULT_MS);
        GifScreen {
            app: app.clone(),
            d,
            item,
            range,
            fps: 15,
            width: 480,
            plays: Plays::Loop,
            scroller: Scroller::new(8.0 * d, d),
            press: None,
            last: None,
            making: None,
            error: None,
        }
    }

    pub fn item_id(&self) -> i64 {
        self.item.id
    }

    pub fn set_range(&mut self, r: ClipRange) {
        self.range = r;
    }

    pub fn back(&mut self) -> bool {
        self.making.is_some()
    }

    pub fn wants_frames(&self) -> bool {
        self.making.is_some()
    }

    fn plan(&self) -> GifPlan {
        GifPlan { start_ms: self.range.start_ms, end_ms: self.range.end_ms, fps: self.fps, width: self.width, plays: self.plays }
    }

    fn lay(&self) -> Lay {
        let d = self.d;
        let preview_y = 16.0 * d;
        let part_y = preview_y + PREVIEW_DP * d + 16.0 * d;
        let fps_y = part_y + (34.0 + 64.0) * d;
        let width_y = fps_y + 86.0 * d;
        let plays_y = width_y + 86.0 * d + 62.0 * d;
        let note_y = plays_y + 3.0 * PLAY_ROW_DP * d + 28.0 * d;
        Lay { preview_y, part_y, fps_y, width_y, plays_y, note_y, height: note_y + 80.0 * d }
    }

    fn top(&self, f: &Frame) -> f32 {
        f.ins.top + BAR_DP * self.d
    }

    fn bottom(&self, f: &Frame) -> f32 {
        f.h - f.ins.bottom - BOTTOM_DP * self.d
    }

    fn xs(&self, f: &Frame) -> (f32, f32) {
        (f.ins.left + 16.0 * self.d, f.w - f.ins.right - 16.0 * self.d)
    }

    fn hit(&self, x: f32, y: f32, now: Instant) -> Option<Hit> {
        let f = self.last?;
        let d = self.d;
        let (top, bottom) = (self.top(&f), self.bottom(&f));
        if y < top {
            return (y >= f.ins.top && x < 56.0 * d).then_some(Hit::Back);
        }
        if y >= bottom {
            return (y < f.h - f.ins.bottom).then_some(if x < f.w / 2.0 { Hit::Cancel } else { Hit::Make });
        }
        let l = self.lay();
        let cy = y - top + self.scroller.offset(now);
        let (x0, x1) = self.xs(&f);
        let col = |n: usize| (((x - x0) / (x1 - x0)) * n as f32).clamp(0.0, n as f32 - 1.0) as usize;
        if cy >= l.part_y + 34.0 * d && cy < l.part_y + 98.0 * d {
            return Some(Hit::Part);
        }
        if cy >= l.fps_y + 34.0 * d && cy < l.fps_y + 78.0 * d {
            return Some(Hit::Fps(col(FPS.len())));
        }
        if cy >= l.width_y + 34.0 * d && cy < l.width_y + 78.0 * d {
            return Some(Hit::Width(col(WIDTHS.len())));
        }
        if cy >= l.plays_y && cy < l.plays_y + 3.0 * PLAY_ROW_DP * d {
            return Some(Hit::Plays(((cy - l.plays_y) / (PLAY_ROW_DP * d)) as usize));
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn segmented(&self, gl: &Context, r: &mut Renderer, batch: &mut Batch, rc: Rect, labels: &[&str], chosen: usize, pressed: Option<usize>) {
        let d = self.d;
        let accent = Rgba::hex(holo::accent());
        batch.outline(rc, Rgba::hex(0x3A3A3A), 0.0, 1.0 * d);
        let cw = rc.w / labels.len() as f32;
        for (i, label) in labels.iter().enumerate() {
            let cell = Rect::new(rc.x + cw * i as f32, rc.y, cw, rc.h);
            if i == chosen {
                batch.rect(cell, accent, 0.0);
            } else if pressed == Some(i) {
                batch.rect(cell, accent.with_alpha(0.3), 0.0);
            }
            if i > 0 {
                batch.rect(Rect::new(cell.x, cell.y, 1.0 * d, cell.h), Rgba::hex(0x3A3A3A), 0.0);
            }
            let em = 13.0 * d;
            let lw = r.measure_spaced(label, em, 0.5 * d, Weight::Medium);
            let c = if i == chosen { Rgba::hex(0x000000) } else { Rgba::hex(WHITE) };
            r.text_spaced(gl, batch, label, cell.x + (cell.w - lw) / 2.0, widgets::baseline(r, cell.y + cell.h / 2.0, em, d), em, 0.5 * d, cell.w, c, Weight::Medium);
        }
    }

    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, f: Frame, now: Instant) -> bool {
        let d = self.d;
        self.last = Some(f);
        let white = Rgba::hex(WHITE);
        let accent = Rgba::hex(holo::accent());
        let l = self.lay();
        let (top, bottom) = (self.top(&f), self.bottom(&f));
        self.scroller.set_bounds((l.height - (bottom - top)).max(0.0));
        let oy = top - self.scroller.offset(now);
        let (x0, x1) = self.xs(&f);
        let w = x1 - x0;
        batch.rect(Rect::new(0.0, 0.0, f.w, f.h), Rgba::hex(0x000000), 0.0);

        // The preview: the video's picture at the GIF's shape, sized by width.
        let (vw, vh) = gif_maker::upright(&self.item);
        let (gw, gh) = self.plan().size(vw.max(1), vh.max(1));
        let aspect = gw as f32 / gh.max(1) as f32;
        let full_px = if vw > 0 { vw as f32 } else { 1920.0 };
        let share = (gw as f32 / full_px).clamp(0.0, 1.0);
        let max_h = PREVIEW_DP * d;
        let pw = ((120.0 * d + share * (w - 120.0 * d)).min(max_h * aspect)).min(w);
        let ph = pw / aspect;
        let pr = Rect::new(x0 + (w - pw) / 2.0, oy + l.preview_y + (max_h - ph) / 2.0, pw, ph);
        match r.icon_uv(&thumbs::key(self.item.id)) {
            Some([u0, v0, u1, v1]) => {
                // The thumbnail is square: show its middle at the GIF's shape.
                let (du, dv) = if aspect >= 1.0 { (0.0, (v1 - v0) * (1.0 - 1.0 / aspect) / 2.0) } else { ((u1 - u0) * (1.0 - aspect) / 2.0, 0.0) };
                batch.image(pr, [u0 + du, v0 + dv, u1 - du, v1 - dv], 0.0, 1.0);
            }
            None => batch.rect(pr, Rgba::hex(widgets::TILE_EMPTY), 0.0),
        }
        let em = 12.0 * d;
        let bw = r.measure("GIF", em, f.w, Weight::Medium) + 12.0 * d;
        let badge = Rect::new(pr.x + pr.w - 10.0 * d - bw, pr.y + pr.h - 10.0 * d - 20.0 * d, bw, 20.0 * d);
        batch.rect(badge, Rgba::hex(0x000000).with_alpha(0.7), 0.0);
        r.text(gl, batch, "GIF", badge.x + 6.0 * d, widgets::baseline(r, badge.y + badge.h / 2.0, em, d), em, bw, white, Weight::Medium);

        // PART OF THE VIDEO.
        widgets::heading(gl, r, batch, x0, oy + l.part_y, w, "PART OF THE VIDEO", "", d);
        let py = oy + l.part_y + 34.0 * d;
        if self.press == Some(Hit::Part) {
            batch.rect(Rect::new(x0, py, w, 64.0 * d), accent.with_alpha(0.3), 0.0);
        }
        let len = self.range.len_ms();
        let part = format!("{} to {} · {}.{} s", time_label(self.range.start_ms), time_label(self.range.end_ms), len / 1000, len / 100 % 10);
        r.text(gl, batch, &part, x0 + 4.0 * d, py + 26.0 * d, 17.0 * d, w, white, Weight::Regular);
        r.text(gl, batch, "Tap to choose a different part", x0 + 4.0 * d, py + 47.0 * d, 13.0 * d, w, Rgba::hex(SUB), Weight::Regular);
        batch.rect(Rect::new(x0, py + 64.0 * d - 1.0 * d, w, 1.0 * d), Rgba::hex(LINE), 0.0);

        // Frames a second, width.
        let pressed = |h: fn(usize) -> Hit, n: usize| (0..n).find(|&i| self.press == Some(h(i)));
        r.text(gl, batch, "Frames a second", x0 + 4.0 * d, oy + l.fps_y + 24.0 * d, 14.0 * d, w, Rgba::hex(QUIET), Weight::Regular);
        let fps_labels: Vec<String> = FPS.iter().map(|v| v.to_string()).collect();
        let fps_refs: Vec<&str> = fps_labels.iter().map(String::as_str).collect();
        let chosen = FPS.iter().position(|&v| v == self.fps).unwrap_or(1);
        self.segmented(gl, r, batch, Rect::new(x0, oy + l.fps_y + 34.0 * d, w, 44.0 * d), &fps_refs, chosen, pressed(Hit::Fps, FPS.len()));
        r.text(gl, batch, "Width", x0 + 4.0 * d, oy + l.width_y + 24.0 * d, 14.0 * d, w, Rgba::hex(QUIET), Weight::Regular);
        let labels: Vec<&str> = WIDTHS.iter().map(|w| w.1).collect();
        let chosen = WIDTHS.iter().position(|v| v.0 == self.width).unwrap_or(1);
        self.segmented(gl, r, batch, Rect::new(x0, oy + l.width_y + 34.0 * d, w, 44.0 * d), &labels, chosen, pressed(Hit::Width, WIDTHS.len()));

        // PLAYS.
        widgets::heading(gl, r, batch, x0, oy + l.plays_y - 46.0 * d, w, "PLAYS", "", d);
        for (i, (p, label)) in PLAYS.iter().enumerate() {
            let ry = oy + l.plays_y + i as f32 * PLAY_ROW_DP * d;
            let row = Rect::new(x0, ry, w, PLAY_ROW_DP * d);
            if self.press == Some(Hit::Plays(i)) {
                batch.rect(row, accent.with_alpha(0.3), 0.0);
            }
            r.text(gl, batch, label, x0 + 4.0 * d, widgets::baseline(r, ry + row.h / 2.0, 17.0 * d, d), 17.0 * d, w - 56.0 * d, white, Weight::Regular);
            widgets::radio(batch, x1 - 20.0 * d, ry + row.h / 2.0, self.plays == *p, d);
            batch.rect(Rect::new(x0, ry + row.h - 1.0 * d, w, 1.0 * d), Rgba::hex(LINE), 0.0);
        }

        // The note.
        let bytes = self.plan().estimate_bytes(vw.max(1), vh.max(1));
        let (note, colour) = match &self.error {
            Some(e) => (e.clone(), WARN),
            None if bytes > TOO_BIG => (format!("About {}. That's big for a GIF, and some apps refuse files this size. Fewer frames or a smaller width will shrink it.", format::bytes(bytes)), WARN),
            None => (format!("About {}. Saved as a new GIF; the video isn't changed.", format::bytes(bytes.max(100_000))), SUB),
        };
        for (k, line) in widgets::wrap(r, &note, 14.0 * d, w - 8.0 * d).iter().enumerate() {
            r.text(gl, batch, line, x0 + 4.0 * d, oy + l.note_y + k as f32 * 20.0 * d, 14.0 * d, w, Rgba::hex(colour), Weight::Regular);
        }

        // The bar.
        let (bt, bh) = (f.ins.top, BAR_DP * d);
        batch.rect(Rect::new(0.0, 0.0, f.w, bt + bh), Rgba::hex(0x141414), 0.0);
        batch.rect(Rect::new(0.0, bt + bh - 1.0 * d, f.w, 1.0 * d), Rgba::hex(0x2A2A2A), 0.0);
        if self.press == Some(Hit::Back) {
            batch.rect(Rect::new(0.0, bt, 56.0 * d, bh - 1.0 * d), accent.with_alpha(0.3), 0.0);
        }
        let cy = bt + bh / 2.0;
        holo::stroke(batch, (18.0 * d, cy), (26.0 * d, cy - 8.0 * d), 2.2 * d, white);
        holo::stroke(batch, (18.0 * d, cy), (26.0 * d, cy + 8.0 * d), 2.2 * d, white);
        r.text(gl, batch, "Make a GIF", 60.0 * d, widgets::baseline(r, cy, 20.0 * d, d), 20.0 * d, f.w - 80.0 * d, white, Weight::Regular);

        // Cancel / Make GIF.
        batch.rect(Rect::new(0.0, bottom, f.w, f.h - bottom), Rgba::hex(0x141414), 0.0);
        batch.rect(Rect::new(0.0, bottom, f.w, 1.0 * d), Rgba::hex(0x2A2A2A), 0.0);
        batch.rect(Rect::new(f.w / 2.0, bottom, 1.0 * d, BOTTOM_DP * d), Rgba::hex(0x2A2A2A), 0.0);
        for (i, (label, hit, c, wt)) in [("Cancel", Hit::Cancel, white, Weight::Regular), ("Make GIF", Hit::Make, accent, Weight::Medium)].into_iter().enumerate() {
            let cell = Rect::new(f.w / 2.0 * i as f32, bottom, f.w / 2.0, BOTTOM_DP * d);
            if self.press == Some(hit) {
                batch.rect(cell, accent.with_alpha(0.3), 0.0);
            }
            let em = 15.0 * d;
            let lw = r.measure(label, em, f.w, wt);
            r.text(gl, batch, label, cell.x + (cell.w - lw) / 2.0, widgets::baseline(r, cell.y + cell.h / 2.0, em, d), em, lw + 1.0, c, wt);
        }

        // Making: a dim layer with the progress.
        if let Some(m) = &self.making {
            let (done, n) = (m.done.load(Ordering::Acquire), m.total);
            batch.rect(Rect::new(0.0, 0.0, f.w, f.h), Rgba::hex(0x000000).with_alpha(0.6), 0.0);
            let (bw, bh) = (f.w - 64.0 * d, 96.0 * d);
            let b = Rect::new(32.0 * d, f.h / 2.0 - bh / 2.0, bw, bh);
            batch.rect(b, Rgba::hex(0x282828), 0.0);
            let msg = format!("Making the GIF… {} of {n} frames", done.min(n));
            r.text(gl, batch, &msg, b.x + 20.0 * d, b.y + 38.0 * d, 17.0 * d, bw - 40.0 * d, white, Weight::Regular);
            let track = Rect::new(b.x + 20.0 * d, b.y + 62.0 * d, bw - 40.0 * d, 4.0 * d);
            batch.rect(track, Rgba::hex(0x555555), 0.0);
            batch.rect(Rect::new(track.x, track.y, track.w * done as f32 / n.max(1) as f32, track.h), accent, 0.0);
        }
        self.scroller.animating(now) || self.making.is_some()
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        if self.making.is_some() {
            return Action::None;
        }
        let Some(f) = self.last else { return Action::None };
        let (top, bottom) = (self.top(&f), self.bottom(&f));
        let in_content = |y: f32| y >= top && y < bottom;
        match t.kind {
            TouchKind::Down => {
                self.press = self.hit(t.x, t.y, t.t);
                if in_content(t.y) {
                    self.scroller.touch_down(t.x, t.y, t.t);
                }
            }
            TouchKind::Move => {
                self.scroller.touch_move(t.x, t.y, t.t);
                if self.scroller.moved() {
                    self.press = None;
                }
            }
            TouchKind::Up => {
                let tap = matches!(self.scroller.touch_up(t.x, t.y, t.t), ListGesture::Tap { .. });
                let hit = self.press.take();
                if hit.is_some() && (tap || !in_content(t.y)) && self.hit(t.x, t.y, t.t) == hit {
                    return self.pressed(hit.unwrap());
                }
            }
            TouchKind::Cancel | TouchKind::SecondDown => {
                self.press = None;
                self.scroller.touch_cancel(t.t);
            }
            TouchKind::SecondUp => {}
        }
        Action::None
    }

    fn pressed(&mut self, h: Hit) -> Action {
        match h {
            Hit::Back | Hit::Cancel => return Action::Close,
            Hit::Part => return Action::ChoosePart(self.range),
            Hit::Fps(i) => self.fps = FPS[i.min(FPS.len() - 1)],
            Hit::Width(i) => self.width = WIDTHS[i.min(WIDTHS.len() - 1)].0,
            Hit::Plays(i) => self.plays = PLAYS[i.min(PLAYS.len() - 1)].0,
            Hit::Make => {
                self.error = None;
                self.making = Some(gif_maker::start(&self.app, self.item.clone(), self.plan()));
            }
        }
        Action::None
    }

    /// Finished making, once.
    pub fn poll(&mut self) -> Action {
        let Some(m) = &self.making else { return Action::None };
        let Some(r) = m.result.lock().unwrap().take() else { return Action::None };
        self.making = None;
        match r {
            Ok(name) => Action::Done(name),
            Err(e) => {
                self.error = Some(e);
                Action::None
            }
        }
    }
}
