//! Convert pictures, as the approved mockup: a bar with the count; FROM
//! (a few thumbnails, what kinds, total size); SAVE AS formats; Quality;
//! Picture size; Remove location; Keep date and camera details; the size
//! guess; Cancel and Convert. Converting runs one picture at a time on its
//! own thread, carries on past failures, and says how it went.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use android_activity::AndroidApp;
use gallery_core::convert::{estimate, summary, Format, SizeChoice};
use gallery_core::crop::Crop;
use gallery_core::format;
use gallery_core::media::MediaItem;
use glow::Context;
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, WHITE};
use holo_platform::jni_util::with_activity;
use holo_platform::render::Renderer;
use jni::objects::JValue;
use jni::{jni_sig, jni_str};
use launcher_core::scroll::{ListGesture, Scroller};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::crop_save::{self, java_format, Output};
use crate::input::{Touch, TouchKind};
use crate::photos::Frame;
use crate::{saving, thumbs, widgets};

const BAR_DP: f32 = 56.0;
const BOTTOM_DP: f32 = 56.0;
const FORMAT_ROW_DP: f32 = 66.0;
const TICK_ROW_DP: f32 = 56.0;
const QUIET: u32 = 0xBDBDBD;
const SUB: u32 = 0x9E9E9E;
const OFF: u32 = 0x6E6E6E;
const LINE: u32 = 0x262626;
/// Biggest picture we'll make: 100 megapixels.
const MAX_PIXELS: u64 = 100_000_000;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Close,
    /// Finished: what to tell the owner.
    Done(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Back,
    Format(usize),
    Quality,
    Size(usize),
    NoLocation,
    KeepDetails,
    Cancel,
    Convert,
}

/// Where the rows sit, in content pixels (0 = just under the bar).
struct Lay {
    from_y: f32,
    formats_y: f32,
    quality_y: Option<f32>,
    size_y: f32,
    ticks_y: f32,
    note_y: f32,
    height: f32,
}

#[derive(Default)]
struct Progress {
    done: usize,
    failed: usize,
    finished: bool,
}

pub struct ConvertScreen {
    app: AndroidApp,
    d: f32,
    items: Vec<MediaItem>,
    /// SAVE AS… from Crop: the crop and the picture's upright size.
    crop: Option<(Crop, (u32, u32))>,
    /// Videos and GIFs in the selection that were left out.
    skipped: usize,
    can_heic: bool,
    format: Format,
    quality: u8,
    size: SizeChoice,
    no_location: bool,
    keep_details: bool,
    scroller: Scroller,
    press: Option<Hit>,
    sliding: bool,
    last: Option<Frame>,
    running: Option<Arc<Mutex<Progress>>>,
}

impl ConvertScreen {
    pub fn new(app: &AndroidApp, d: f32, items: Vec<MediaItem>, crop: Option<(Crop, (u32, u32))>, skipped: usize) -> ConvertScreen {
        let can_heic = with_activity(app, |env, a| env.call_method(a, jni_str!("canHeic"), jni_sig!("()Z"), &[])?.z()).unwrap_or(false);
        ConvertScreen {
            app: app.clone(),
            d,
            items,
            crop,
            skipped,
            can_heic,
            format: Format::Jpeg,
            quality: 85,
            size: SizeChoice::Original,
            no_location: true,
            keep_details: true,
            scroller: Scroller::new(8.0 * d, d),
            press: None,
            sliding: false,
            last: None,
            running: None,
        }
    }

    fn available(&self, f: Format) -> bool {
        match f {
            Format::Heic => self.can_heic,
            // No AVIF writer on Android yet.
            Format::Avif => false,
            _ => true,
        }
    }

    fn about(&self, f: Format) -> &'static str {
        match f {
            Format::Heic if !self.can_heic => "This phone can't make HEIC pictures.",
            Format::Heic => "Smaller than JPEG. This phone can make it.",
            Format::Avif => "This phone can't make AVIF pictures.",
            _ => f.about(),
        }
    }

    pub fn back(&mut self) -> bool {
        self.running.is_some()
    }

    pub fn wants_frames(&self) -> bool {
        self.running.is_some()
    }

    fn lay(&self) -> Lay {
        let d = self.d;
        let from_y = 16.0 * d;
        // FROM heading and its row, then the SAVE AS heading.
        let formats_y = from_y + (34.0 + 76.0 + 18.0 + 34.0) * d;
        let after_formats = formats_y + Format::ALL.len() as f32 * FORMAT_ROW_DP * d;
        let quality_y = self.format.has_quality().then_some(after_formats);
        let size_y = after_formats + if quality_y.is_some() { 74.0 * d } else { 0.0 };
        let ticks_y = size_y + 90.0 * d;
        let note_y = ticks_y + 2.0 * TICK_ROW_DP * d + 28.0 * d;
        Lay { from_y, formats_y, quality_y, size_y, ticks_y, note_y, height: note_y + 80.0 * d }
    }

    fn content_top(&self, f: &Frame) -> f32 {
        f.ins.top + BAR_DP * self.d
    }

    fn content_bottom(&self, f: &Frame) -> f32 {
        f.h - f.ins.bottom - BOTTOM_DP * self.d
    }

    fn x0(&self, f: &Frame) -> f32 {
        f.ins.left + 16.0 * self.d
    }

    fn x1(&self, f: &Frame) -> f32 {
        f.w - f.ins.right - 16.0 * self.d
    }

    /// The quality slider's track: x from, x to, and its centre line (content y).
    fn slider(&self, f: &Frame, l: &Lay) -> Option<(f32, f32, f32)> {
        let y = l.quality_y?;
        Some((self.x0(f) + 12.0 * self.d, self.x1(f) - 12.0 * self.d, y + 50.0 * self.d))
    }

    fn total_bytes(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }

    fn hit(&self, x: f32, y: f32, now: Instant) -> Option<Hit> {
        let f = self.last?;
        let d = self.d;
        let top = self.content_top(&f);
        let bottom = self.content_bottom(&f);
        if y < top {
            return (y >= f.ins.top && x < 56.0 * d).then_some(Hit::Back);
        }
        if y >= bottom {
            if y >= f.h - f.ins.bottom {
                return None;
            }
            return Some(if x < f.w / 2.0 { Hit::Cancel } else { Hit::Convert });
        }
        let l = self.lay();
        let cy = y - top + self.scroller.offset(now);
        let (x0, x1) = (self.x0(&f), self.x1(&f));
        if x < x0 - 8.0 * d || x > x1 + 8.0 * d {
            return None;
        }
        if cy >= l.formats_y && cy < l.formats_y + Format::ALL.len() as f32 * FORMAT_ROW_DP * d {
            return Some(Hit::Format(((cy - l.formats_y) / (FORMAT_ROW_DP * d)) as usize));
        }
        if let Some((_, _, sy)) = self.slider(&f, &l)
            && (cy - sy).abs() < 22.0 * d
        {
            return Some(Hit::Quality);
        }
        let seg_y = l.size_y + 34.0 * d;
        if cy >= seg_y && cy < seg_y + 44.0 * d {
            let i = (((x - x0) / (x1 - x0)) * 3.0).clamp(0.0, 2.0) as usize;
            return Some(Hit::Size(i));
        }
        if cy >= l.ticks_y && cy < l.ticks_y + 2.0 * TICK_ROW_DP * d {
            return Some(if cy < l.ticks_y + TICK_ROW_DP * d { Hit::NoLocation } else { Hit::KeepDetails });
        }
        None
    }

    fn set_quality_at(&mut self, x: f32) {
        let Some(f) = self.last else { return };
        let Some((a, b, _)) = self.slider(&f, &self.lay()) else { return };
        let v = 40.0 + ((x - a) / (b - a)).clamp(0.0, 1.0) * 60.0;
        self.quality = ((v / 5.0).round() * 5.0) as u8;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, f: Frame, now: Instant) -> (bool, Vec<(i64, bool, u16)>) {
        let d = self.d;
        self.last = Some(f);
        let white = Rgba::hex(WHITE);
        let accent = Rgba::hex(holo::accent());
        let l = self.lay();
        let top = self.content_top(&f);
        let bottom = self.content_bottom(&f);
        self.scroller.set_bounds((l.height - (bottom - top)).max(0.0));
        let oy = top - self.scroller.offset(now);
        let (x0, x1) = (self.x0(&f), self.x1(&f));
        let w = x1 - x0;
        let mut missing = Vec::new();
        batch.rect(Rect::new(0.0, 0.0, f.w, f.h), Rgba::hex(0x000000), 0.0);

        // FROM: a few thumbnails, what kinds, how big.
        let from = if self.skipped > 0 { format!("FROM ({} left out: not pictures)", self.skipped) } else { "FROM".into() };
        widgets::heading(gl, r, batch, x0, oy + l.from_y, w, &from, "", d);
        let ty = oy + l.from_y + 46.0 * d;
        let ts = 52.0 * d;
        let shown = self.items.iter().take(3).count();
        for (i, it) in self.items.iter().take(3).enumerate() {
            let rc = Rect::new(x0 + 4.0 * d + i as f32 * (ts + 8.0 * d), ty, ts, ts);
            match r.icon_uv(&thumbs::key(it.id)) {
                Some(uv) => batch.image(rc, uv, 0.0, 1.0),
                None => {
                    batch.rect(rc, Rgba::hex(widgets::TILE_EMPTY), 0.0);
                    missing.push((it.id, false, it.orientation));
                }
            }
        }
        let tx = x0 + 4.0 * d + shown as f32 * (ts + 8.0 * d) + 6.0 * d;
        let mimes: Vec<&str> = self.items.iter().map(|i| i.mime.as_str()).collect();
        let kinds = if self.crop.is_some() { format!("{} (cropped)", summary(&mimes)) } else { summary(&mimes) };
        r.text(gl, batch, &kinds, tx, ty + 22.0 * d, 16.0 * d, x1 - tx, white, Weight::Regular);
        let total = format!("{} in total", format::bytes(self.total_bytes()));
        r.text(gl, batch, &total, tx, ty + 42.0 * d, 13.0 * d, x1 - tx, Rgba::hex(SUB), Weight::Regular);
        batch.rect(Rect::new(x0, ty + ts + 12.0 * d, w, 1.0 * d), Rgba::hex(LINE), 0.0);

        // SAVE AS.
        widgets::heading(gl, r, batch, x0, oy + l.formats_y - 40.0 * d, w, "SAVE AS", "", d);
        for (i, fmt) in Format::ALL.iter().enumerate() {
            let ry = oy + l.formats_y + i as f32 * FORMAT_ROW_DP * d;
            let row = Rect::new(x0, ry, w, FORMAT_ROW_DP * d);
            let on = self.available(*fmt);
            if self.press == Some(Hit::Format(i)) && on {
                batch.rect(row, accent.with_alpha(0.3), 0.0);
            }
            let (c, sc) = if on { (white, Rgba::hex(SUB)) } else { (Rgba::hex(OFF), Rgba::hex(OFF)) };
            let tw = w - 56.0 * d;
            r.text(gl, batch, fmt.name(), x0 + 4.0 * d, ry + 26.0 * d, 17.0 * d, tw, c, Weight::Regular);
            for (k, line) in widgets::wrap(r, self.about(*fmt), 13.0 * d, tw).iter().take(2).enumerate() {
                r.text(gl, batch, line, x0 + 4.0 * d, ry + 45.0 * d + k as f32 * 16.0 * d, 13.0 * d, tw, sc, Weight::Regular);
            }
            let (cx, cy) = (x1 - 20.0 * d, ry + row.h / 2.0);
            if on {
                widgets::radio(batch, cx, cy, self.format == *fmt, d);
            } else {
                batch.outline(Rect::new(cx - 8.0 * d, cy - 8.0 * d, 16.0 * d, 16.0 * d), Rgba::hex(0x4A4A4A), 8.0 * d, 2.0 * d);
            }
            batch.rect(Rect::new(x0, ry + row.h - 1.0 * d, w, 1.0 * d), Rgba::hex(LINE), 0.0);
        }

        // Quality.
        if let (Some(qy), Some((a, b, sy))) = (l.quality_y, self.slider(&f, &l)) {
            let y = oy + qy;
            r.text(gl, batch, "Quality", x0 + 4.0 * d, y + 28.0 * d, 14.0 * d, 200.0 * d, Rgba::hex(QUIET), Weight::Regular);
            let v = self.quality.to_string();
            let vw = r.measure(&v, 14.0 * d, f.w, Weight::Regular);
            r.text(gl, batch, &v, x1 - 4.0 * d - vw, y + 28.0 * d, 14.0 * d, vw + 1.0, white, Weight::Regular);
            let sy = oy + sy;
            batch.rect(Rect::new(a, sy - 1.0 * d, b - a, 2.0 * d), Rgba::hex(0x555555), 0.0);
            let tx = a + (self.quality as f32 - 40.0) / 60.0 * (b - a);
            batch.rect(Rect::new(a, sy - 1.5 * d, tx - a, 3.0 * d), accent, 0.0);
            batch.circle(tx, sy, 9.0 * d, accent);
        }

        // Picture size.
        let y = oy + l.size_y;
        r.text(gl, batch, "Picture size", x0 + 4.0 * d, y + 22.0 * d, 14.0 * d, w, Rgba::hex(QUIET), Weight::Regular);
        let seg = Rect::new(x0, y + 34.0 * d, w, 44.0 * d);
        batch.outline(seg, Rgba::hex(0x3A3A3A), 0.0, 1.0 * d);
        let cw = seg.w / 3.0;
        for (i, s) in SizeChoice::ALL.iter().enumerate() {
            let cell = Rect::new(seg.x + cw * i as f32, seg.y, cw, seg.h);
            let on = *s == self.size;
            if on {
                batch.rect(cell, accent, 0.0);
            } else if self.press == Some(Hit::Size(i)) {
                batch.rect(cell, accent.with_alpha(0.3), 0.0);
            }
            if i > 0 {
                batch.rect(Rect::new(cell.x, cell.y, 1.0 * d, cell.h), Rgba::hex(0x3A3A3A), 0.0);
            }
            let em = 13.0 * d;
            let lw = r.measure_spaced(s.label(), em, 0.5 * d, Weight::Medium);
            let c = if on { Rgba::hex(0x000000) } else { white };
            r.text_spaced(gl, batch, s.label(), cell.x + (cell.w - lw) / 2.0, widgets::baseline(r, cell.y + cell.h / 2.0, em, d), em, 0.5 * d, cell.w, c, Weight::Medium);
        }

        // The two ticks.
        for (i, (label, on, hit)) in [("Remove location", self.no_location, Hit::NoLocation), ("Keep date and camera details", self.keep_details, Hit::KeepDetails)].into_iter().enumerate() {
            let ry = oy + l.ticks_y + i as f32 * TICK_ROW_DP * d;
            let row = Rect::new(x0, ry, w, TICK_ROW_DP * d);
            if self.press == Some(hit) {
                batch.rect(row, accent.with_alpha(0.3), 0.0);
            }
            r.text(gl, batch, label, x0 + 4.0 * d, widgets::baseline(r, ry + row.h / 2.0, 17.0 * d, d), 17.0 * d, w - 60.0 * d, white, Weight::Regular);
            widgets::tick_box(batch, x1 - 20.0 * d, ry + row.h / 2.0, on, d);
            batch.rect(Rect::new(x0, ry + row.h - 1.0 * d, w, 1.0 * d), Rgba::hex(LINE), 0.0);
        }

        // The guess.
        let guess = estimate(self.total_bytes(), self.format, self.size, self.quality);
        let copies = if self.items.len() == 1 { "The new copy goes next to the original" } else { "New copies go next to the originals" };
        let note = format!("{copies}, about {} in total. The originals stay as they are.", format::bytes(guess.max(100_000)));
        for (k, line) in widgets::wrap(r, &note, 14.0 * d, w - 8.0 * d).iter().enumerate() {
            r.text(gl, batch, line, x0 + 4.0 * d, oy + l.note_y + k as f32 * 20.0 * d, 14.0 * d, w, Rgba::hex(SUB), Weight::Regular);
        }

        // The bar over what scrolled under it.
        let bt = f.ins.top;
        let bh = BAR_DP * d;
        batch.rect(Rect::new(0.0, 0.0, f.w, bt + bh), Rgba::hex(0x141414), 0.0);
        batch.rect(Rect::new(0.0, bt + bh - 1.0 * d, f.w, 1.0 * d), Rgba::hex(0x2A2A2A), 0.0);
        if self.press == Some(Hit::Back) {
            batch.rect(Rect::new(0.0, bt, 56.0 * d, bh - 1.0 * d), accent.with_alpha(0.3), 0.0);
        }
        let cy = bt + bh / 2.0;
        holo::stroke(batch, (18.0 * d, cy), (26.0 * d, cy - 8.0 * d), 2.2 * d, white);
        holo::stroke(batch, (18.0 * d, cy), (26.0 * d, cy + 8.0 * d), 2.2 * d, white);
        let title = if self.items.len() == 1 { "Convert 1 picture".to_string() } else { format!("Convert {} pictures", format::grouped(self.items.len())) };
        r.text(gl, batch, &title, 60.0 * d, widgets::baseline(r, cy, 20.0 * d, d), 20.0 * d, f.w - 80.0 * d, white, Weight::Regular);

        // Cancel / Convert.
        batch.rect(Rect::new(0.0, bottom, f.w, f.h - bottom), Rgba::hex(0x141414), 0.0);
        batch.rect(Rect::new(0.0, bottom, f.w, 1.0 * d), Rgba::hex(0x2A2A2A), 0.0);
        batch.rect(Rect::new(f.w / 2.0, bottom, 1.0 * d, BOTTOM_DP * d), Rgba::hex(0x2A2A2A), 0.0);
        for (i, (label, hit, c, wt)) in [("Cancel", Hit::Cancel, white, Weight::Regular), ("Convert", Hit::Convert, accent, Weight::Medium)].into_iter().enumerate() {
            let cell = Rect::new(f.w / 2.0 * i as f32, bottom, f.w / 2.0, BOTTOM_DP * d);
            if self.press == Some(hit) {
                batch.rect(cell, accent.with_alpha(0.3), 0.0);
            }
            let em = 15.0 * d;
            let lw = r.measure(label, em, f.w, wt);
            r.text(gl, batch, label, cell.x + (cell.w - lw) / 2.0, widgets::baseline(r, cell.y + cell.h / 2.0, em, d), em, lw + 1.0, c, wt);
        }

        // Running: a dim layer with the progress.
        if let Some(p) = &self.running {
            let (done, n) = (p.lock().unwrap().done, self.items.len());
            batch.rect(Rect::new(0.0, 0.0, f.w, f.h), Rgba::hex(0x000000).with_alpha(0.6), 0.0);
            let (bw, bh) = (f.w - 64.0 * d, 96.0 * d);
            let b = Rect::new(32.0 * d, f.h / 2.0 - bh / 2.0, bw, bh);
            batch.rect(b, Rgba::hex(0x282828), 0.0);
            let msg = if n == 1 { "Converting…".to_string() } else { format!("Converting {} of {n}…", (done + 1).min(n)) };
            r.text(gl, batch, &msg, b.x + 20.0 * d, b.y + 38.0 * d, 17.0 * d, bw - 40.0 * d, white, Weight::Regular);
            let track = Rect::new(b.x + 20.0 * d, b.y + 62.0 * d, bw - 40.0 * d, 4.0 * d);
            batch.rect(track, Rgba::hex(0x555555), 0.0);
            batch.rect(Rect::new(track.x, track.y, track.w * done as f32 / n.max(1) as f32, track.h), accent, 0.0);
        }
        (self.scroller.animating(now) || self.running.is_some(), missing)
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        if self.running.is_some() {
            return Action::None;
        }
        let Some(f) = self.last else { return Action::None };
        match t.kind {
            TouchKind::Down => {
                self.press = self.hit(t.x, t.y, t.t);
                self.sliding = self.press == Some(Hit::Quality);
                if self.sliding {
                    self.set_quality_at(t.x);
                } else if t.y >= self.content_top(&f) && t.y < self.content_bottom(&f) {
                    self.scroller.touch_down(t.x, t.y, t.t);
                }
            }
            TouchKind::Move => {
                if self.sliding {
                    self.set_quality_at(t.x);
                    return Action::None;
                }
                self.scroller.touch_move(t.x, t.y, t.t);
                if self.scroller.moved() {
                    self.press = None;
                }
            }
            TouchKind::Up => {
                if std::mem::take(&mut self.sliding) {
                    self.press = None;
                    return Action::None;
                }
                let tap = matches!(self.scroller.touch_up(t.x, t.y, t.t), ListGesture::Tap { .. });
                let hit = self.press.take();
                let in_content = t.y >= self.content_top(&f) && t.y < self.content_bottom(&f);
                if hit.is_some() && (tap || !in_content) && self.hit(t.x, t.y, t.t) == hit {
                    return self.pressed(hit.unwrap());
                }
            }
            TouchKind::Cancel | TouchKind::SecondDown => {
                self.press = None;
                self.sliding = false;
                self.scroller.touch_cancel(t.t);
            }
            TouchKind::SecondUp => {}
        }
        Action::None
    }

    fn pressed(&mut self, h: Hit) -> Action {
        match h {
            Hit::Back | Hit::Cancel => return Action::Close,
            Hit::Convert => self.start(),
            Hit::Format(i) => {
                let fmt = Format::ALL[i.min(Format::ALL.len() - 1)];
                if self.available(fmt) {
                    self.format = fmt;
                }
            }
            Hit::Quality => {}
            Hit::Size(i) => self.size = SizeChoice::ALL[i.min(2)],
            Hit::NoLocation => self.no_location = !self.no_location,
            Hit::KeepDetails => self.keep_details = !self.keep_details,
        }
        Action::None
    }

    fn start(&mut self) {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let out = Output { format: self.format, quality: self.quality, size: self.size, keep_details: self.keep_details, no_location: self.no_location };
        let (app, items, crop, slot) = (self.app.clone(), self.items.clone(), self.crop, progress.clone());
        let spawned = std::thread::Builder::new().name("convert".into()).spawn(move || {
            let t0 = Instant::now();
            for it in &items {
                let r = match crop {
                    Some((c, full)) => crop_save::save_as(&app, it, c, full, Some(out)),
                    None => convert_one(&app, it, out),
                };
                let mut p = slot.lock().unwrap();
                p.done += 1;
                if let Err(e) = r {
                    log::warn!("convert {}: {e}", it.name);
                    p.failed += 1;
                }
                drop(p);
                holo_platform::waker::wake();
            }
            let mut p = slot.lock().unwrap();
            log::info!("convert: {} to {:?}, {} failed, {:.1} s", items.len(), out.format, p.failed, t0.elapsed().as_secs_f64());
            p.finished = true;
            drop(p);
            crate::library::refresh();
            holo_platform::waker::wake();
        });
        match spawned {
            Ok(_) => self.running = Some(progress),
            Err(e) => log::warn!("convert thread: {e}"),
        }
    }

    /// Finished converting, once: the message for the owner.
    pub fn poll(&mut self) -> Action {
        let Some(p) = &self.running else { return Action::None };
        let p = p.lock().unwrap();
        if !p.finished {
            return Action::None;
        }
        let ok = p.done - p.failed;
        let msg = match (ok, p.failed) {
            (_, 0) if ok == 1 => "Converted 1 picture".to_string(),
            (_, 0) => format!("Converted {} pictures", format::grouped(ok)),
            (0, f) => format!("Couldn't convert {}", if f == 1 { "it".to_string() } else { format!("any of {f}") }),
            (o, f) => format!("Converted {o}, {f} failed"),
        };
        drop(p);
        self.running = None;
        Action::Done(msg)
    }
}

/// Convert one whole picture into a new file next to it.
fn convert_one(app: &AndroidApp, it: &MediaItem, out: Output) -> Result<String, String> {
    let sideways = it.orientation == 90 || it.orientation == 270;
    let (w, h) = if sideways { (it.h, it.w) } else { (it.w, it.h) };
    let (tw, th) = if w > 0 && h > 0 { out.size.apply(w, h) } else { (0, 0) };
    if tw as u64 * th as u64 > MAX_PIXELS {
        return Err("too big".into());
    }
    let copy = saving::begin(app, it.id, &it.name, false, "_conv", out.format.ext(), out.format.mime(), it.when_taken(), false).ok_or("Couldn't make the new file")?;
    let fd = copy.open_fd("rw").ok_or("Couldn't write the new file")?;
    let args = [
        JValue::Long(it.id),
        JValue::Int(it.orientation as i32),
        JValue::Int(tw as i32),
        JValue::Int(th as i32),
        JValue::Int(java_format(out.format)),
        JValue::Int(out.quality as i32),
        JValue::Int(fd),
        JValue::Bool(out.keep_details),
        JValue::Bool(out.no_location),
    ];
    let ok = with_activity(app, |env, a| env.call_method(a, jni_str!("editConvert"), jni_sig!("(JIIIIIIZZ)Z"), &args)?.z()).map_err(|e| e.to_string())?;
    if !ok {
        return Err("couldn't convert".into());
    }
    let name = copy.name.clone();
    if copy.finish() { Ok(name) } else { Err("couldn't finish".into()) }
}
