//! Crop and rotate, as the approved mockup: a bar with X, RESET and SAVE
//! COPY; the picture with its frame; Straighten; the shapes; ROTATE, FLIP
//! and SAVE AS…. Saving runs on its own thread.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use android_activity::AndroidApp;
use gallery_core::crop::{max_inside, turned_size, Crop, Shape};
use gallery_core::media::MediaItem;
use glow::{Context, HasContext};
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, WHITE};
use holo_platform::render::Renderer;
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::input::{Touch, TouchKind};
use crate::photo_tex::{Tex, Upload};
use crate::photos::Frame;
use crate::viewer_jobs::{Done, Job, Jobs};
use crate::widgets;

const BAR_DP: f32 = 56.0;
const PANEL_DP: f32 = 212.0;
const SLIDER_DP: f32 = 70.0;
const SHAPES_DP: f32 = 52.0;
const BUTTONS_DP: f32 = 84.0;
const BAR_BG: u32 = 0x141414;
const LINE: u32 = 0x2A2A2A;
const QUIET: u32 = 0xBDBDBD;
const OFF: u32 = 0x8A8A8A;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Close,
    /// Saved; the new file's name.
    Saved(String),
    /// SAVE AS…: convert this picture with the crop applied.
    SaveAs(Crop),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Btn {
    Close,
    Reset,
    Save,
    Shape(usize),
    Rotate,
    Flip,
    SaveAs,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    Corner(u8),
    Frame,
    Slider,
}

type Outcome = Arc<Mutex<Option<Result<String, String>>>>;

pub struct CropScreen {
    app: AndroidApp,
    d: f32,
    item: MediaItem,
    crop: Crop,
    /// The picture's upright size, once known.
    full: (u32, u32),
    tex: Option<Tex>,
    upload: Option<Upload>,
    asked: bool,
    drag: Drag,
    at: (f32, f32),
    press: Option<Btn>,
    last: Option<Frame>,
    saving: Option<Outcome>,
    error: Option<(Instant, String)>,
    program: Option<CropDraw>,
}

impl CropScreen {
    pub fn new(app: &AndroidApp, d: f32, item: MediaItem) -> CropScreen {
        let sideways = item.orientation == 90 || item.orientation == 270;
        let full = if sideways { (item.h.max(1), item.w.max(1)) } else { (item.w.max(1), item.h.max(1)) };
        CropScreen {
            app: app.clone(),
            d,
            item,
            crop: Crop::default(),
            full,
            tex: None,
            upload: None,
            asked: false,
            drag: Drag::None,
            at: (0.0, 0.0),
            press: None,
            last: None,
            saving: None,
            error: None,
            program: None,
        }
    }

    pub fn item(&self) -> &MediaItem {
        &self.item
    }

    /// Set up with a crop already chosen (coming back from SAVE AS…).
    pub fn crop(&self) -> Crop {
        self.crop
    }

    /// The picture's upright size.
    pub fn full(&self) -> (u32, u32) {
        self.full
    }

    pub fn free(mut self, gl: &Context) {
        if let Some(t) = self.tex.take() {
            t.free(gl);
        }
        if let Some(u) = self.upload.take() {
            u.cancel(gl);
        }
        if let Some(p) = self.program.take() {
            p.free(gl);
        }
    }

    /// Back closes it, unless a save is running.
    pub fn back(&mut self) -> bool {
        self.saving.is_some()
    }

    pub fn is_saving(&self) -> bool {
        self.saving.is_some()
    }

    fn area(&self, f: &Frame) -> Rect {
        let d = self.d;
        let top = f.ins.top + BAR_DP * d;
        let bottom = f.h - f.ins.bottom - PANEL_DP * d;
        Rect::new(f.ins.left, top, f.w - f.ins.left - f.ins.right, (bottom - top).max(1.0))
    }

    /// Where the turned picture sits: left, top, scale (screen px per picture px).
    fn placement(&self, f: &Frame) -> (f32, f32, f32) {
        let a = self.area(f);
        let m = 24.0 * self.d;
        let (tw, th) = turned_size(self.full.0, self.full.1, self.crop.turns);
        let s = ((a.w - 2.0 * m) / tw as f32).min((a.h - 2.0 * m) / th as f32);
        (a.x + (a.w - tw as f32 * s) / 2.0, a.y + (a.h - th as f32 * s) / 2.0, s)
    }

    fn frame_rect(&self, f: &Frame) -> Rect {
        let (ox, oy, s) = self.placement(f);
        let (tw, th) = turned_size(self.full.0, self.full.1, self.crop.turns);
        let (x, y, w, h) = self.crop.rect;
        Rect::new(ox + x * tw as f32 * s, oy + y * th as f32 * s, w * tw as f32 * s, h * th as f32 * s)
    }

    fn panel_top(&self, f: &Frame) -> f32 {
        f.h - f.ins.bottom - PANEL_DP * self.d
    }

    fn slider_track(&self, f: &Frame) -> (f32, f32, f32) {
        let d = self.d;
        let y = self.panel_top(f) + 14.0 * d + 20.0 * d + 20.0 * d;
        (f.ins.left + 24.0 * d, f.w - f.ins.right - 24.0 * d, y)
    }

    fn shapes_rect(&self, f: &Frame) -> Rect {
        Rect::new(f.ins.left, self.panel_top(f) + (SLIDER_DP + 6.0) * self.d, f.w - f.ins.left - f.ins.right, SHAPES_DP * self.d)
    }

    fn buttons_rect(&self, f: &Frame) -> Rect {
        let s = self.shapes_rect(f);
        Rect::new(s.x, s.y + s.h, s.w, BUTTONS_DP * self.d)
    }

    fn bar_buttons(&self, f: &Frame, r: Option<&mut Renderer>) -> [(Btn, Rect); 3] {
        let d = self.d;
        let (top, h) = (f.ins.top, BAR_DP * d);
        let save_w = r.map_or(130.0 * d, |r| r.measure_spaced("SAVE COPY", 14.0 * d, 0.6 * d, Weight::Medium) + 60.0 * d);
        let save = Rect::new(f.w - f.ins.right - save_w, top, save_w, h);
        let reset = Rect::new(save.x - 80.0 * d, top, 80.0 * d, h);
        [(Btn::Close, Rect::new(f.ins.left, top, 48.0 * d, h)), (Btn::Reset, reset), (Btn::Save, save)]
    }

    fn button_at(&self, x: f32, y: f32) -> Option<Btn> {
        let f = self.last?;
        if let Some((b, _)) = self.bar_buttons(&f, None).into_iter().find(|(_, r)| r.contains(x, y)) {
            return Some(b);
        }
        let s = self.shapes_rect(&f);
        if s.contains(x, y) {
            return Some(Btn::Shape((((x - s.x) / s.w) * Shape::ALL.len() as f32) as usize));
        }
        let b = self.buttons_rect(&f);
        if b.contains(x, y) {
            return Some([Btn::Rotate, Btn::Flip, Btn::SaveAs][(((x - b.x) / b.w) * 3.0) as usize % 3]);
        }
        None
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        let Some(f) = self.last else { return Action::None };
        if self.saving.is_some() {
            return Action::None;
        }
        let d = self.d;
        match t.kind {
            TouchKind::Down => {
                self.at = (t.x, t.y);
                self.press = self.button_at(t.x, t.y);
                self.drag = Drag::None;
                if self.press.is_some() {
                    return Action::None;
                }
                let (x0, x1, sy) = self.slider_track(&f);
                if (t.y - sy).abs() < 24.0 * d && t.x > x0 - 20.0 * d && t.x < x1 + 20.0 * d {
                    self.drag = Drag::Slider;
                    self.set_angle_from(t.x, x0, x1);
                    return Action::None;
                }
                let fr = self.frame_rect(&f);
                let corners = [(fr.x, fr.y), (fr.x + fr.w, fr.y), (fr.x, fr.y + fr.h), (fr.x + fr.w, fr.y + fr.h)];
                let near = corners.iter().enumerate().map(|(i, c)| (i, (c.0 - t.x).hypot(c.1 - t.y))).min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((i, dist)) = near.filter(|n| n.1 < 36.0 * d) {
                    let _ = dist;
                    self.drag = Drag::Corner(i as u8);
                } else if fr.contains(t.x, t.y) {
                    self.drag = Drag::Frame;
                }
            }
            TouchKind::Move => {
                if self.press.is_some() {
                    if self.button_at(t.x, t.y) != self.press {
                        self.press = None;
                    }
                    return Action::None;
                }
                let (dx, dy) = (t.x - self.at.0, t.y - self.at.1);
                self.at = (t.x, t.y);
                let (_, _, s) = self.placement(&f);
                let (tw, th) = turned_size(self.full.0, self.full.1, self.crop.turns);
                let (px, py) = (dx / (tw as f32 * s), dy / (th as f32 * s));
                match self.drag {
                    Drag::Corner(c) => self.crop.drag_corner(c, px, py, self.full.0, self.full.1),
                    Drag::Frame => self.crop.move_by(px, py),
                    Drag::Slider => {
                        let (x0, x1, _) = self.slider_track(&f);
                        self.set_angle_from(t.x, x0, x1);
                    }
                    Drag::None => {}
                }
            }
            TouchKind::Up => {
                self.drag = Drag::None;
                let Some(b) = self.press.take() else { return Action::None };
                if self.button_at(t.x, t.y) != Some(b) {
                    return Action::None;
                }
                let (w, h) = self.full;
                match b {
                    Btn::Close => return Action::Close,
                    Btn::Reset => self.crop = Crop::default(),
                    Btn::Save => self.start_save(),
                    Btn::Shape(i) => self.crop.set_shape(Shape::ALL[i.min(Shape::ALL.len() - 1)], w, h),
                    Btn::Rotate => self.crop.rotate(w, h),
                    Btn::Flip => self.crop.toggle_flip(),
                    Btn::SaveAs => return Action::SaveAs(self.crop),
                }
            }
            TouchKind::Cancel => {
                self.drag = Drag::None;
                self.press = None;
            }
            TouchKind::SecondDown | TouchKind::SecondUp => {}
        }
        Action::None
    }

    fn set_angle_from(&mut self, x: f32, x0: f32, x1: f32) {
        let a = (((x - x0) / (x1 - x0)).clamp(0.0, 1.0) * 90.0 - 45.0).round();
        // A little pull towards straight.
        self.crop.angle_deg = if a.abs() <= 1.0 { 0.0 } else { a };
    }

    fn start_save(&mut self) {
        let out: Outcome = Arc::default();
        let (app, item, crop, full, o) = (self.app.clone(), self.item.clone(), self.crop, self.full, out.clone());
        std::thread::Builder::new()
            .name("crop-save".into())
            .spawn(move || {
                let r = crate::crop_save::save(&app, &item, crop, full);
                if let Err(e) = &r {
                    log::warn!("crop save: {e}");
                }
                *o.lock().unwrap() = Some(r);
                holo_platform::waker::wake();
            })
            .expect("crop-save thread");
        self.saving = Some(out);
    }

    /// The saving thread finished: what to do now.
    pub fn poll(&mut self) -> Action {
        let Some(r) = self.saving.as_ref().and_then(|o| o.lock().unwrap().take()) else { return Action::None };
        self.saving = None;
        match r {
            Ok(name) => Action::Saved(name),
            Err(e) => {
                self.error = Some((Instant::now(), e));
                Action::None
            }
        }
    }

    pub fn wants_frames(&self) -> bool {
        self.upload.is_some() || self.saving.is_some() || self.error.as_ref().is_some_and(|e| e.0.elapsed().as_secs() < 3)
    }

    /// The picture (drawn straight onto the screen, before the batch).
    pub fn draw_picture(&mut self, gl: &Context, jobs: &Jobs, f: Frame, screen: (i32, i32), now: Instant) {
        self.last = Some(f);
        if !self.asked {
            self.asked = true;
            let max = (f.w as u32, f.h as u32);
            jobs.want(vec![Job::Full { id: self.item.id, orientation: self.item.orientation, max }]);
        }
        for done in jobs.take() {
            if let Done::Full { id, w, h, full, rgba } = done
                && id == self.item.id
                && self.tex.is_none()
            {
                if full.0 > 0 && full.1 > 0 {
                    self.full = full;
                }
                self.upload = Upload::start(gl, w, h, rgba);
            }
        }
        if let Some(u) = self.upload.take() {
            match u.step(gl, now + std::time::Duration::from_millis(4)) {
                Ok(t) => self.tex = Some(t),
                Err(u) => self.upload = Some(u),
            }
        }
        crate::photo_tex::PhotoDraw::clear(gl, screen);
        if self.tex.is_none() {
            return;
        }
        if self.program.is_none() {
            self.program = Some(CropDraw::new(gl));
        }
        let (ox, oy, s) = self.placement(&f);
        let (w, h) = self.full;
        let (tw, th) = turned_size(w, h, self.crop.turns);
        let (tw, th) = (tw as f32, th as f32);
        let k = max_inside(self.crop.angle_deg, tw, th);
        let a = self.crop.angle_deg.to_radians();
        let (c, sn) = (a.cos(), a.sin());
        let (cx, cy) = (ox + tw * s / 2.0, oy + th * s / 2.0);
        let plain = Crop { turns: self.crop.turns, flip: self.crop.flip, ..Crop::default() };
        let mut pos = [0f32; 8];
        let mut uv = [0f32; 8];
        for (i, (ux, uy)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].into_iter().enumerate() {
            let (vx, vy) = ((ux - 0.5) * tw * s * k, (uy - 0.5) * th * s * k);
            pos[i * 2] = cx + vx * c - vy * sn;
            pos[i * 2 + 1] = cy + vx * sn + vy * c;
            let (sx, sy) = plain.source_point(ux * tw, uy * th, w, h);
            uv[i * 2] = sx / w as f32;
            uv[i * 2 + 1] = sy / h as f32;
        }
        let area = self.area(&f);
        if let (Some(program), Some(tex)) = (self.program.as_ref(), self.tex.as_ref()) {
            program.draw(gl, tex, &pos, &uv, screen, area);
        }
    }

    /// Bars, frame and controls.
    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, f: Frame, now: Instant) -> bool {
        let d = self.d;
        self.last = Some(f);
        let white = Rgba::hex(WHITE);
        let accent = Rgba::hex(holo::accent());
        let area = self.area(&f);
        if self.tex.is_some() {
            // Dim everything outside the frame.
            let fr = self.frame_rect(&f);
            let dim = Rgba::hex(0x000000).with_alpha(0.62);
            batch.rect(Rect::new(area.x, area.y, area.w, (fr.y - area.y).max(0.0)), dim, 0.0);
            batch.rect(Rect::new(area.x, fr.y + fr.h, area.w, (area.y + area.h - fr.y - fr.h).max(0.0)), dim, 0.0);
            batch.rect(Rect::new(area.x, fr.y, (fr.x - area.x).max(0.0), fr.h), dim, 0.0);
            batch.rect(Rect::new(fr.x + fr.w, fr.y, (area.x + area.w - fr.x - fr.w).max(0.0), fr.h), dim, 0.0);
            batch.outline(fr, white.with_alpha(0.9), 0.0, 1.0 * d);
            for k in [1.0 / 3.0, 2.0 / 3.0] {
                batch.rect(Rect::new(fr.x + fr.w * k, fr.y, 1.0 * d, fr.h), white.with_alpha(0.45), 0.0);
                batch.rect(Rect::new(fr.x, fr.y + fr.h * k, fr.w, 1.0 * d), white.with_alpha(0.45), 0.0);
            }
            let (l, t) = (22.0 * d, 4.0 * d);
            for (i, (x, y)) in [(fr.x, fr.y), (fr.x + fr.w, fr.y), (fr.x, fr.y + fr.h), (fr.x + fr.w, fr.y + fr.h)].into_iter().enumerate() {
                let (sx, sy) = (if i % 2 == 0 { 1.0 } else { -1.0 }, if i < 2 { 1.0 } else { -1.0 });
                let (ax, ay) = (x - sx * t, y - sy * t);
                batch.rect(Rect::new(ax.min(ax + sx * l), ay.min(ay + sy * t), l, t), accent, 0.0);
                batch.rect(Rect::new(ax.min(ax + sx * t), ay.min(ay + sy * l), t, l), accent, 0.0);
            }
        } else {
            batch.rect(area, Rgba::hex(0x0A0A0A), 0.0);
        }

        // Cover anything the turned picture drew outside its area.
        batch.rect(Rect::new(0.0, 0.0, f.w, area.y), Rgba::hex(0x000000), 0.0);
        batch.rect(Rect::new(0.0, area.y + area.h, f.w, f.h - area.y - area.h), Rgba::hex(0x000000), 0.0);

        // The bar.
        let (top, bh) = (f.ins.top, BAR_DP * d);
        batch.rect(Rect::new(0.0, 0.0, f.w, top + bh), Rgba::hex(BAR_BG), 0.0);
        batch.rect(Rect::new(0.0, top + bh - 2.0 * d, f.w, 2.0 * d), accent, 0.0);
        let buttons = self.bar_buttons(&f, Some(r));
        for (b, rc) in &buttons {
            if self.press == Some(*b) {
                batch.rect(*rc, accent.with_alpha(0.3), 0.0);
            }
        }
        let cy = top + bh / 2.0;
        let close = buttons[0].1;
        let (xc, yc) = (close.x + close.w / 2.0, cy);
        holo::stroke(batch, (xc - 7.0 * d, yc - 7.0 * d), (xc + 7.0 * d, yc + 7.0 * d), 2.2 * d, white);
        holo::stroke(batch, (xc + 7.0 * d, yc - 7.0 * d), (xc - 7.0 * d, yc + 7.0 * d), 2.2 * d, white);
        batch.rect(Rect::new(close.x + close.w, top, 1.0 * d, bh - 2.0 * d), Rgba::hex(LINE), 0.0);
        let em = 20.0 * d;
        r.text(gl, batch, "Crop", close.x + close.w + 16.0 * d, widgets::baseline(r, cy, em, d), em, 200.0 * d, white, Weight::Regular);
        let (reset, save) = (buttons[1].1, buttons[2].1);
        let lem = 14.0 * d;
        let rw = r.measure_spaced("RESET", lem, 0.6 * d, Weight::Medium);
        r.text_spaced(gl, batch, "RESET", reset.x + (reset.w - rw) / 2.0, widgets::baseline(r, cy, lem, d), lem, 0.6 * d, reset.w, Rgba::hex(QUIET), Weight::Medium);
        batch.rect(Rect::new(save.x, top, 1.0 * d, bh - 2.0 * d), Rgba::hex(LINE), 0.0);
        let (kx, ky) = (save.x + 16.0 * d, cy);
        holo::stroke(batch, (kx, ky), (kx + 5.0 * d, ky + 5.0 * d), 2.4 * d, accent);
        holo::stroke(batch, (kx + 5.0 * d, ky + 5.0 * d), (kx + 16.0 * d, ky - 6.0 * d), 2.4 * d, accent);
        r.text_spaced(gl, batch, "SAVE COPY", kx + 26.0 * d, widgets::baseline(r, cy, lem, d), lem, 0.6 * d, save.w, white, Weight::Medium);

        // Straighten.
        let pt = self.panel_top(&f);
        let label_y = pt + 14.0 * d + 14.0 * d;
        r.text(gl, batch, "Straighten", f.ins.left + 16.0 * d, label_y, 14.0 * d, 200.0 * d, Rgba::hex(QUIET), Weight::Regular);
        let value = format!("{}°", self.crop.angle_deg as i32);
        let vw = r.measure(&value, 14.0 * d, f.w, Weight::Regular);
        r.text(gl, batch, &value, f.w - f.ins.right - 16.0 * d - vw, label_y, 14.0 * d, vw + 1.0, white, Weight::Regular);
        let (x0, x1, sy) = self.slider_track(&f);
        batch.rect(Rect::new(x0, sy - 1.0 * d, x1 - x0, 2.0 * d), Rgba::hex(0x555555), 0.0);
        let mid = (x0 + x1) / 2.0;
        let tx = x0 + (self.crop.angle_deg + 45.0) / 90.0 * (x1 - x0);
        batch.rect(Rect::new(mid.min(tx), sy - 1.5 * d, (tx - mid).abs(), 3.0 * d), accent, 0.0);
        batch.rect(Rect::new(mid - 0.5 * d, sy - 6.0 * d, 1.0 * d, 12.0 * d), Rgba::hex(OFF), 0.0);
        batch.circle(tx, sy, 9.0 * d, accent);

        // Shapes.
        let s = self.shapes_rect(&f);
        batch.rect(s, Rgba::hex(0x0D0D0D), 0.0);
        batch.rect(Rect::new(s.x, s.y, s.w, 1.0 * d), Rgba::hex(LINE), 0.0);
        batch.rect(Rect::new(s.x, s.y + s.h - 1.0 * d, s.w, 1.0 * d), Rgba::hex(LINE), 0.0);
        let cw = s.w / Shape::ALL.len() as f32;
        for (i, shape) in Shape::ALL.iter().enumerate() {
            let x = s.x + cw * i as f32;
            if self.press == Some(Btn::Shape(i)) {
                batch.rect(Rect::new(x, s.y, cw, s.h), accent.with_alpha(0.3), 0.0);
            }
            let on = *shape == self.crop.shape;
            let em = 13.0 * d;
            let lw = r.measure_spaced(shape.label(), em, 0.5 * d, Weight::Medium);
            r.text_spaced(gl, batch, shape.label(), x + (cw - lw) / 2.0, widgets::baseline(r, s.y + s.h / 2.0 - 1.5 * d, em, d), em, 0.5 * d, cw, if on { white } else { Rgba::hex(OFF) }, Weight::Medium);
            if on {
                batch.rect(Rect::new(x, s.y + s.h - 3.0 * d, cw, 3.0 * d), accent, 0.0);
            }
        }

        // ROTATE, FLIP, SAVE AS…
        let b = self.buttons_rect(&f);
        let bw = b.w / 3.0;
        for (i, (btn, label)) in [(Btn::Rotate, "ROTATE"), (Btn::Flip, "FLIP"), (Btn::SaveAs, "SAVE AS…")].into_iter().enumerate() {
            let x = b.x + bw * i as f32;
            if self.press == Some(btn) {
                batch.rect(Rect::new(x, b.y, bw, b.h), accent.with_alpha(0.3), 0.0);
            }
            let c = if btn == Btn::Flip && self.crop.flip { accent } else { white };
            let (ix, iy, u) = (x + bw / 2.0 - 13.0 * d, b.y + 18.0 * d, d * 26.0 / 24.0);
            let p = |px: f32, py: f32| (ix + px * u, iy + py * u);
            let lw = 1.8 * d;
            match btn {
                Btn::Rotate => {
                    for (a, z) in [((9.0, 13.0), (20.0, 13.0)), ((20.0, 13.0), (20.0, 21.0)), ((20.0, 21.0), (9.0, 21.0)), ((9.0, 21.0), (9.0, 13.0)), ((4.0, 10.0), (8.0, 6.0)), ((8.0, 6.0), (16.0, 5.0)), ((4.0, 4.0), (4.0, 10.0)), ((4.0, 10.0), (10.0, 10.0))] {
                        holo::stroke(batch, p(a.0, a.1), p(z.0, z.1), lw, c);
                    }
                }
                Btn::Flip => {
                    for (a, z) in [((12.0, 3.0), (12.0, 21.0)), ((9.0, 6.0), (3.0, 18.0)), ((3.0, 18.0), (9.0, 18.0)), ((9.0, 18.0), (9.0, 6.0)), ((15.0, 6.0), (21.0, 18.0)), ((21.0, 18.0), (15.0, 18.0)), ((15.0, 18.0), (15.0, 6.0))] {
                        holo::stroke(batch, p(a.0, a.1), p(z.0, z.1), lw, c);
                    }
                }
                _ => {
                    for (a, z) in [((4.0, 8.0), (17.0, 8.0)), ((17.0, 8.0), (14.0, 5.0)), ((20.0, 16.0), (7.0, 16.0)), ((7.0, 16.0), (10.0, 19.0))] {
                        holo::stroke(batch, p(a.0, a.1), p(z.0, z.1), lw, c);
                    }
                }
            }
            let em = 12.0 * d;
            let w = r.measure_spaced(label, em, 0.6 * d, Weight::Medium);
            r.text_spaced(gl, batch, label, x + (bw - w) / 2.0, b.y + 62.0 * d, em, 0.6 * d, bw, c, Weight::Medium);
        }

        // Saving, or why it failed.
        let note = if self.saving.is_some() {
            Some("Saving…".to_string())
        } else {
            self.error.as_ref().filter(|e| now.duration_since(e.0).as_secs() < 3).map(|e| e.1.clone())
        };
        if let Some(msg) = note {
            let em = 15.0 * d;
            let mw = r.measure(&msg, em, f.w, Weight::Regular);
            let (w, h) = (mw + 48.0 * d, 48.0 * d);
            let rc = Rect::new((f.w - w) / 2.0, area.y + (area.h - h) / 2.0, w, h);
            batch.rect(rc, Rgba::hex(0x222222).with_alpha(0.95), 4.0 * d);
            batch.outline(rc, Rgba::hex(0x3A3A3A), 4.0 * d, 1.0 * d);
            r.text(gl, batch, &msg, rc.x + 24.0 * d, widgets::baseline(r, rc.y + h / 2.0, em, d), em, mw + 1.0, white, Weight::Regular);
        }
        self.drag != Drag::None || self.wants_frames()
    }
}

/// Draws the picture as any four-cornered shape (turned and straightened).
struct CropDraw {
    program: glow::Program,
    vao: glow::VertexArray,
    u_screen: Option<glow::UniformLocation>,
    u_pos: Option<glow::UniformLocation>,
    u_uv: Option<glow::UniformLocation>,
}

const VS: &str = r#"#version 300 es
uniform vec2 u_screen;
uniform vec2 u_pos[4];
uniform vec2 u_uv[4];
out vec2 v_uv;
void main() {
    vec2 p = u_pos[gl_VertexID];
    v_uv = u_uv[gl_VertexID];
    gl_Position = vec4(p.x / u_screen.x * 2.0 - 1.0, 1.0 - p.y / u_screen.y * 2.0, 0.0, 1.0);
}"#;

const FS: &str = r#"#version 300 es
precision mediump float;
uniform sampler2D u_tex;
in vec2 v_uv;
out vec4 o;
void main() { o = texture(u_tex, v_uv); }"#;

impl CropDraw {
    fn new(gl: &Context) -> CropDraw {
        unsafe {
            let program = gl.create_program().expect("program");
            for (kind, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
                let sh = gl.create_shader(kind).expect("shader");
                gl.shader_source(sh, src);
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    log::error!("crop shader: {}", gl.get_shader_info_log(sh));
                }
                gl.attach_shader(program, sh);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                log::error!("crop link: {}", gl.get_program_info_log(program));
            }
            gl.use_program(Some(program));
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_tex").as_ref(), 0);
            CropDraw {
                u_screen: gl.get_uniform_location(program, "u_screen"),
                u_pos: gl.get_uniform_location(program, "u_pos"),
                u_uv: gl.get_uniform_location(program, "u_uv"),
                vao: gl.create_vertex_array().expect("vao"),
                program,
            }
        }
    }

    fn draw(&self, gl: &Context, t: &Tex, pos: &[f32; 8], uv: &[f32; 8], screen: (i32, i32), clip: Rect) {
        unsafe {
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(clip.x as i32, screen.1 - (clip.y + clip.h) as i32, clip.w as i32, clip.h as i32);
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.uniform_2_f32(self.u_screen.as_ref(), screen.0 as f32, screen.1 as f32);
            gl.uniform_2_f32_slice(self.u_pos.as_ref(), pos);
            gl.uniform_2_f32_slice(self.u_uv.as_ref(), uv);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(t.raw()));
            gl.disable(glow::BLEND);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.disable(glow::SCISSOR_TEST);
        }
    }

    fn free(self, gl: &Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
        }
    }
}
