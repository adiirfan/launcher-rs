//! "Save as video" for a GIF: a small Holo box over the viewer with the
//! quality choice (no separate screen), then the progress while it saves.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use android_activity::AndroidApp;
use gallery_core::gif_video::Quality;
use gallery_core::media::MediaItem;
use glow::Context;
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, WHITE};
use holo_platform::render::Renderer;
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::gif_maker::Making;
use crate::input::{Touch, TouchKind};
use crate::photos::Frame;
use crate::{gif_to_video, widgets};

const CARD: u32 = 0x282828;
const SUB: u32 = 0x9E9E9E;
const WARN: u32 = 0xFFBB33;
const LINE: u32 = 0x3A3A3A;
const TITLE_DP: f32 = 64.0;
const ROW_DP: f32 = 64.0;
const NOTE_DP: f32 = 56.0;
const BUTTONS_DP: f32 = 48.0;

const CHOICES: [(Quality, &str, &str); 2] = [
    (Quality::Good, "Good quality", "Looks the same as the GIF"),
    (Quality::Smaller, "Smaller file", "Easier to send; a little softer"),
];

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Close,
    /// Saved: the new video's name.
    Done(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Choice(usize),
    Cancel,
    Save,
    Outside,
}

pub struct SaveVideo {
    app: AndroidApp,
    d: f32,
    item: MediaItem,
    q: Quality,
    press: Option<Hit>,
    last: Option<Frame>,
    making: Option<Arc<Making>>,
    error: Option<String>,
}

impl SaveVideo {
    pub fn new(app: &AndroidApp, d: f32, item: MediaItem) -> SaveVideo {
        SaveVideo { app: app.clone(), d, item, q: Quality::Good, press: None, last: None, making: None, error: None }
    }

    /// Back closes the box, but not while it's saving.
    pub fn back(&mut self) -> bool {
        self.making.is_some()
    }

    pub fn wants_frames(&self) -> bool {
        self.making.is_some()
    }

    fn card(&self, f: &Frame) -> Rect {
        let d = self.d;
        let w = (f.w - 32.0 * d).min(360.0 * d);
        let h = if self.making.is_some() { 112.0 * d } else { (TITLE_DP + 2.0 * ROW_DP + NOTE_DP + BUTTONS_DP) * d };
        Rect::new((f.w - w) / 2.0, (f.h - h) / 2.0, w, h)
    }

    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let f = self.last?;
        let d = self.d;
        let c = self.card(&f);
        if x < c.x || x >= c.x + c.w || y < c.y || y >= c.y + c.h {
            return Some(Hit::Outside);
        }
        let rows = c.y + TITLE_DP * d;
        for i in 0..CHOICES.len() {
            let ry = rows + i as f32 * ROW_DP * d;
            if y >= ry && y < ry + ROW_DP * d {
                return Some(Hit::Choice(i));
            }
        }
        let by = c.y + c.h - BUTTONS_DP * d;
        if y >= by {
            return Some(if x < c.x + c.w / 2.0 { Hit::Cancel } else { Hit::Save });
        }
        None
    }

    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, f: Frame, _now: Instant) -> bool {
        let d = self.d;
        self.last = Some(f);
        let white = Rgba::hex(WHITE);
        let accent = Rgba::hex(holo::accent());
        batch.rect(Rect::new(0.0, 0.0, f.w, f.h), Rgba::hex(0x000000).with_alpha(0.6), 0.0);
        let c = self.card(&f);
        batch.rect(c, Rgba::hex(CARD), 0.0);
        let (x0, w) = (c.x + 20.0 * d, c.w - 40.0 * d);

        if let Some(m) = &self.making {
            let (done, n) = (m.done.load(Ordering::Acquire), m.total);
            r.text(gl, batch, "Saving the video…", x0, c.y + 44.0 * d, 17.0 * d, w, white, Weight::Regular);
            let track = Rect::new(x0, c.y + 68.0 * d, w, 4.0 * d);
            batch.rect(track, Rgba::hex(0x555555), 0.0);
            batch.rect(Rect::new(track.x, track.y, track.w * done.min(n) as f32 / n.max(1) as f32, track.h), accent, 0.0);
            return true;
        }

        // Title, with the Holo accent line under it.
        r.text(gl, batch, "Save as video", x0, widgets::baseline(r, c.y + TITLE_DP * d / 2.0, 20.0 * d, d), 20.0 * d, w, accent, Weight::Regular);
        batch.rect(Rect::new(c.x, c.y + TITLE_DP * d - 2.0 * d, c.w, 2.0 * d), accent, 0.0);

        for (i, (q, label, sub)) in CHOICES.iter().enumerate() {
            let ry = c.y + (TITLE_DP + i as f32 * ROW_DP) * d;
            let row = Rect::new(c.x, ry, c.w, ROW_DP * d);
            if self.press == Some(Hit::Choice(i)) {
                batch.rect(row, accent.with_alpha(0.3), 0.0);
            }
            r.text(gl, batch, label, x0, ry + 28.0 * d, 17.0 * d, w - 40.0 * d, white, Weight::Regular);
            r.text(gl, batch, sub, x0, ry + 48.0 * d, 13.0 * d, w - 40.0 * d, Rgba::hex(SUB), Weight::Regular);
            widgets::radio(batch, c.x + c.w - 32.0 * d, ry + row.h / 2.0, self.q == *q, d);
        }

        let ny = c.y + (TITLE_DP + 2.0 * ROW_DP) * d;
        let (note, colour) = match &self.error {
            Some(e) => (e.as_str(), WARN),
            None => ("Saved as a new MP4; the GIF isn't changed.", SUB),
        };
        for (k, line) in widgets::wrap(r, note, 14.0 * d, w).iter().enumerate().take(2) {
            r.text(gl, batch, line, x0, ny + 24.0 * d + k as f32 * 20.0 * d, 14.0 * d, w, Rgba::hex(colour), Weight::Regular);
        }

        // Cancel | Save.
        let by = c.y + c.h - BUTTONS_DP * d;
        batch.rect(Rect::new(c.x, by, c.w, 1.0 * d), Rgba::hex(LINE), 0.0);
        batch.rect(Rect::new(c.x + c.w / 2.0, by, 1.0 * d, BUTTONS_DP * d), Rgba::hex(LINE), 0.0);
        for (i, (label, hit)) in [("Cancel", Hit::Cancel), ("Save", Hit::Save)].into_iter().enumerate() {
            let cell = Rect::new(c.x + c.w / 2.0 * i as f32, by, c.w / 2.0, BUTTONS_DP * d);
            if self.press == Some(hit) {
                batch.rect(cell, accent.with_alpha(0.3), 0.0);
            }
            let em = 15.0 * d;
            let lw = r.measure(label, em, c.w, Weight::Regular);
            r.text(gl, batch, label, cell.x + (cell.w - lw) / 2.0, widgets::baseline(r, cell.y + cell.h / 2.0, em, d), em, lw + 1.0, white, Weight::Regular);
        }
        false
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        if self.making.is_some() {
            return Action::None;
        }
        match t.kind {
            TouchKind::Down => self.press = self.hit(t.x, t.y),
            TouchKind::Up => {
                let hit = self.press.take();
                if hit.is_some() && self.hit(t.x, t.y) == hit {
                    return self.pressed(hit.unwrap());
                }
            }
            TouchKind::Move => {
                if self.press.is_some() && self.hit(t.x, t.y) != self.press {
                    self.press = None;
                }
            }
            TouchKind::Cancel | TouchKind::SecondDown => self.press = None,
            TouchKind::SecondUp => {}
        }
        Action::None
    }

    fn pressed(&mut self, h: Hit) -> Action {
        match h {
            Hit::Outside | Hit::Cancel => return Action::Close,
            Hit::Choice(i) => self.q = CHOICES[i.min(CHOICES.len() - 1)].0,
            Hit::Save => {
                self.error = None;
                self.making = Some(gif_to_video::start(&self.app, self.item.clone(), self.q));
            }
        }
        Action::None
    }

    /// Finished saving, once.
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
