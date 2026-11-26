//! Clip a video, as the approved mockup: X, "Clip video", SAVE COPY; the
//! video with a play button; a strip of frames with two handles; start,
//! length and end; Exact or Instant; Keep sound; the size note. Saving
//! runs on its own thread with the phone's video chip.

use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use android_activity::AndroidApp;
use gallery_core::clip::{estimate_bytes, length_label, time_label, ClipRange};
use gallery_core::format;
use gallery_core::media::MediaItem;
use glow::{Context, HasContext};
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, WHITE};
use holo_platform::jni_util::with_activity;
use holo_platform::render::Renderer;
use jni::objects::{JClass, JIntArray, JString, JValue};
use jni::sys::jint;
use jni::{jni_sig, jni_str, EnvUnowned};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::input::{Touch, TouchKind};
use crate::photos::Frame;
use crate::video::{self, VideoDraw};
use crate::{saving, thumbs, widgets};

const BAR_DP: f32 = 56.0;
const STRIP_DP: f32 = 60.0;
const HANDLE_DP: f32 = 14.0;
const ROW_DP: f32 = 68.0;
const SOUND_ROW_DP: f32 = 60.0;
const FRAMES: usize = 10;
const QUIET: u32 = 0xBDBDBD;
const SUB: u32 = 0x9E9E9E;
const ROW_LINE: u32 = 0x262626;
/// Seek the preview at most this often while a handle moves.
const SEEK_EVERY: Duration = Duration::from_millis(120);
const CANT_EXACT: &str = "This video can't be re-saved exactly; use Instant";

const MODES: [(&str, &str); 2] = [
    ("Exact", "Cuts right where you put the handles. Takes a few seconds for each minute of video."),
    ("Instant", "No quality loss and done at once, but it may start up to 2 seconds early."),
];

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Close,
    /// Saved; the new file's name.
    Saved(String),
    /// Choosing a part for something else (a GIF): this one.
    Picked(ClipRange),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Btn {
    Close,
    Save,
    Play,
    Mode(usize),
    Sound,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Start,
    End,
}

/// Strip frames from Java: (generation, index, RGBA square).
static STRIP: Mutex<Vec<(u32, usize, Vec<u8>)>> = Mutex::new(Vec::new());
static STRIP_GEN: AtomicU32 = AtomicU32::new(0);
static PROGRESS: AtomicI32 = AtomicI32::new(0);

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onStripFrame<'l>(mut env: EnvUnowned<'l>, _class: JClass<'l>, generation: jint, index: jint, w: jint, h: jint, argb: JIntArray<'l>) {
    let size = thumbs::SIZE as i32;
    if w != size || h != size || generation as u32 != STRIP_GEN.load(Ordering::Acquire) {
        return;
    }
    let outcome = env.with_env(|env| -> jni::errors::Result<()> {
        let mut px = vec![0i32; (w * h) as usize];
        argb.get_region(env, 0, &mut px)?;
        let rgba: Vec<u8> = px.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]).collect();
        STRIP.lock().unwrap().push((generation as u32, index as usize, rgba));
        Ok(())
    });
    outcome.resolve::<jni::errors::LogErrorAndDefault>();
    holo_platform::waker::wake();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onClipProgress<'l>(_env: EnvUnowned<'l>, _class: JClass<'l>, percent: jint) {
    if PROGRESS.swap(percent, Ordering::AcqRel) != percent {
        holo_platform::waker::wake();
    }
}

type Outcome = Arc<Mutex<Option<Result<String, String>>>>;

/// Where everything sits.
struct Lay {
    preview: Rect,
    /// The frames, between the handles' outer edges.
    strip: Rect,
    labels_y: f32,
    heading_y: f32,
    rows: [Rect; 3],
    note_y: f32,
}

pub struct ClipScreen {
    app: AndroidApp,
    d: f32,
    item: MediaItem,
    range: ClipRange,
    exact: bool,
    keep_sound: bool,
    // The preview player.
    tex: Option<glow::Texture>,
    video_draw: Option<VideoDraw>,
    opened: bool,
    prepared: bool,
    playing: bool,
    tm: Option<[f32; 16]>,
    pos: u64,
    want_seek: Option<u64>,
    last_seek: Instant,
    // The strip.
    generation: u32,
    frames: Vec<Option<Vec<u8>>>,
    // Touch.
    press: Option<Btn>,
    drag: Option<Drag>,
    grab_dx: f32,
    last: Option<Frame>,
    bar: (Rect, Rect),
    saving: Option<Outcome>,
    note: Option<String>,
    /// The strip area last kept free of the system's Back swipe.
    excluded: Option<Rect>,
    /// Only choosing a part (for a GIF): no CUT choices, DONE instead of SAVE COPY.
    picking: bool,
}

/// Keep Android's edge swipes off `r` (None: give the edges back).
fn exclude_gestures(app: &AndroidApp, r: Option<Rect>) {
    let v: Vec<i32> = r.map_or(Vec::new(), |r| vec![r.x as i32, r.y as i32, r.w.ceil() as i32, r.h.ceil() as i32]);
    let res = with_activity(app, |env, a| {
        let arr = env.new_int_array(v.len())?;
        arr.set_region(env, 0, &v)?;
        env.call_method(a, jni_str!("setGestureExclusion"), jni_sig!("([I)V"), &[JValue::Object(&arr)]).map(|_| ())
    });
    if let Err(e) = res {
        log::warn!("gesture exclusion: {e}");
    }
}

impl ClipScreen {
    pub fn new(app: &AndroidApp, d: f32, item: MediaItem) -> ClipScreen {
        let generation = STRIP_GEN.fetch_add(1, Ordering::AcqRel) + 1;
        STRIP.lock().unwrap().clear();
        let id = item.id;
        let r = with_activity(app, |env, a| {
            let args = [JValue::Long(id), JValue::Int(FRAMES as i32), JValue::Int(thumbs::SIZE as i32), JValue::Int(generation as i32)];
            env.call_method(a, jni_str!("clipStrip"), jni_sig!("(JIII)V"), &args).map(|_| ())
        });
        if let Err(e) = r {
            log::warn!("clip strip: {e}");
        }
        ClipScreen {
            app: app.clone(),
            d,
            range: ClipRange::new(item.duration_ms),
            item,
            exact: true,
            keep_sound: true,
            tex: None,
            video_draw: None,
            opened: false,
            prepared: false,
            playing: false,
            tm: None,
            pos: 0,
            want_seek: None,
            last_seek: Instant::now(),
            generation,
            frames: vec![None; FRAMES],
            press: None,
            drag: None,
            grab_dx: 0.0,
            last: None,
            bar: (Rect::new(0.0, 0.0, 0.0, 0.0), Rect::new(0.0, 0.0, 0.0, 0.0)),
            saving: None,
            note: None,
            excluded: None,
            picking: false,
        }
    }

    /// Choose a part of the video for something else, starting from `range`.
    pub fn picker(app: &AndroidApp, d: f32, item: MediaItem, range: ClipRange) -> ClipScreen {
        let mut s = ClipScreen::new(app, d, item);
        s.range = range;
        s.picking = true;
        s
    }

    pub fn free(mut self, gl: &Context) {
        if self.excluded.is_some() {
            exclude_gestures(&self.app, None);
        }
        if self.opened {
            video::close(&self.app);
        }
        if let Some(t) = self.tex.take() {
            unsafe { gl.delete_texture(t) };
        }
        if let Some(v) = self.video_draw.take() {
            v.free(gl);
        }
    }

    /// Back closes it, unless a save is running.
    pub fn back(&mut self) -> bool {
        self.saving.is_some()
    }

    pub fn wants_frames(&self) -> bool {
        self.saving.is_some() || self.playing || self.want_seek.is_some()
    }

    fn lay(&self, f: &Frame) -> Lay {
        let d = self.d;
        let top = f.ins.top + BAR_DP * d;
        let bottom = f.h - f.ins.bottom;
        // Below the preview: strip, labels, CUT, two choices, Keep sound, the note.
        let rest = (18.0 + STRIP_DP + 34.0 + 44.0 + 2.0 * ROW_DP + SOUND_ROW_DP + 70.0) * d;
        let ph = (bottom - top - rest).clamp(120.0 * d, 232.0 * d);
        let preview = Rect::new(0.0, top, f.w, ph);
        let x0 = f.ins.left + 16.0 * d;
        let x1 = f.w - f.ins.right - 16.0 * d;
        let sy = top + ph + 18.0 * d;
        let strip = Rect::new(x0 + HANDLE_DP * d, sy, x1 - x0 - 2.0 * HANDLE_DP * d, STRIP_DP * d);
        let labels_y = sy + STRIP_DP * d + 26.0 * d;
        let heading_y = labels_y + 18.0 * d;
        let r0 = heading_y + 40.0 * d;
        let rw = x1 - x0;
        let rows = [Rect::new(x0, r0, rw, ROW_DP * d), Rect::new(x0, r0 + ROW_DP * d, rw, ROW_DP * d), Rect::new(x0, r0 + 2.0 * ROW_DP * d, rw, SOUND_ROW_DP * d)];
        Lay { preview, strip, labels_y, heading_y, rows, note_y: r0 + (2.0 * ROW_DP + SOUND_ROW_DP) * d + 26.0 * d }
    }

    fn ms_to_x(&self, s: &Rect, ms: u64) -> f32 {
        s.x + s.w * ms as f32 / self.range.dur_ms.max(1) as f32
    }

    fn x_to_ms(&self, s: &Rect, x: f32) -> u64 {
        (((x - s.x) / s.w).clamp(0.0, 1.0) * self.range.dur_ms as f32) as u64
    }

    fn seek(&mut self, ms: u64) {
        self.want_seek = Some(ms);
    }

    /// The video and nothing else, straight onto the screen before the batch.
    pub fn draw_preview(&mut self, gl: &Context, f: Frame, screen: (i32, i32), now: Instant) {
        self.last = Some(f);
        crate::photo_tex::PhotoDraw::clear(gl, screen);
        if !self.opened {
            self.opened = true;
            if let Some(t) = video::new_texture(gl) {
                video::open(&self.app, self.item.id, t);
                self.tex = Some(t);
            }
        }
        for e in video::take_events() {
            match e {
                video::Event::Prepared(..) => {
                    self.prepared = true;
                    self.seek(self.range.start_ms);
                }
                video::Event::Completed | video::Event::Paused => self.playing = false,
                video::Event::Error => self.note = Some("Can't play this video here, but it can still be clipped".into()),
            }
        }
        if self.prepared {
            if let Some(ms) = self.want_seek
                && now >= self.last_seek + SEEK_EVERY
            {
                video::seek(&self.app, ms);
                self.pos = ms;
                self.want_seek = None;
                self.last_seek = now;
            }
            if self.playing {
                self.pos = video::position(&self.app).0;
                if self.pos >= self.range.end_ms {
                    video::pause(&self.app);
                    self.playing = false;
                    self.seek(self.range.start_ms);
                }
            }
        }
        if video::frame_waiting()
            && let Some(tm) = video::update(&self.app)
        {
            self.tm = Some(tm);
        }
        if let (Some(tex), Some(tm)) = (self.tex, self.tm) {
            let dest = self.video_rect(&self.lay(&f).preview);
            let vd = self.video_draw.get_or_insert_with(|| VideoDraw::new(gl).expect("video program"));
            vd.draw(gl, tex, (dest.x, dest.y, dest.w, dest.h), &tm, screen);
        }
    }

    /// The video fitted into the preview, upright.
    fn video_rect(&self, p: &Rect) -> Rect {
        let it = &self.item;
        let (w, h) = if it.orientation == 90 || it.orientation == 270 { (it.h, it.w) } else { (it.w, it.h) };
        let (w, h) = (w.max(1) as f32, h.max(1) as f32);
        let s = (p.w / w).min(p.h / h);
        Rect::new(p.x + (p.w - w * s) / 2.0, p.y + (p.h - h * s) / 2.0, w * s, h * s)
    }

    fn play_rect(&self, l: &Lay) -> Rect {
        let s = 60.0 * self.d;
        Rect::new(l.preview.x + (l.preview.w - s) / 2.0, l.preview.y + (l.preview.h - s) / 2.0, s, s)
    }

    /// Everything else. Returns whether it wants another frame.
    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, f: Frame, _now: Instant) -> bool {
        let d = self.d;
        self.last = Some(f);
        let white = Rgba::hex(WHITE);
        let accent = Rgba::hex(holo::accent());
        let l = self.lay(&f);

        // The thumbnail stands in until the first frame arrives.
        if self.tm.is_none()
            && let Some(uv) = r.icon_uv(&thumbs::key(self.item.id))
        {
            let v = self.video_rect(&l.preview);
            let side = v.w.min(v.h);
            batch.image(Rect::new(v.x + (v.w - side) / 2.0, v.y + (v.h - side) / 2.0, side, side), uv, 0.0, 1.0);
        }
        if !self.playing {
            let p = self.play_rect(&l);
            let bg = if self.press == Some(Btn::Play) { accent.with_alpha(0.5) } else { Rgba::hex(0x000000).with_alpha(0.5) };
            batch.circle(p.x + p.w / 2.0, p.y + p.h / 2.0, p.w / 2.0, bg);
            batch.outline(p, white, p.w / 2.0, 2.0 * d);
            widgets::play_triangle(batch, p.x + p.w / 2.0 - 7.0 * d, p.y + p.h / 2.0, 22.0 * d, white);
        }
        let label = time_label(self.pos);
        let em = 13.0 * d;
        let lw = r.measure(&label, em, f.w, Weight::Regular);
        let lr = Rect::new(l.preview.x + 10.0 * d, l.preview.y + l.preview.h - 30.0 * d, lw + 12.0 * d, 22.0 * d);
        batch.rect(lr, Rgba::hex(0x000000).with_alpha(0.7), 0.0);
        r.text(gl, batch, &label, lr.x + 6.0 * d, widgets::baseline(r, lr.y + lr.h / 2.0, em, d), em, lw + 1.0, white, Weight::Regular);

        // The strip: frames, dimmed outside the clip, accent edges and handles.
        let s = l.strip;
        let grab = Rect::new(0.0, s.y - 16.0 * d, f.w, s.h + 32.0 * d);
        if self.excluded != Some(grab) {
            exclude_gestures(&self.app, Some(grab));
            self.excluded = Some(grab);
        }
        let fy = s.y + 2.0 * d;
        let fh = s.h - 4.0 * d;
        let cw = s.w / FRAMES as f32;
        self.take_frames();
        for i in 0..FRAMES {
            let cell = Rect::new(s.x + cw * i as f32, fy, cw - 1.0 * d, fh);
            let key = format!("strip:{}:{}", self.item.id, i);
            let uv = match &self.frames[i] {
                Some(rgba) => r.icon(gl, &key, rgba, thumbs::SIZE, thumbs::SIZE),
                None => None,
            };
            match uv {
                Some([u0, v0, u1, v1]) => {
                    // The middle of the square, shaped like the cell.
                    let k = (cell.w / cell.h).min(1.0);
                    let du = (u1 - u0) * (1.0 - k) / 2.0;
                    batch.image(cell, [u0 + du, v0, u1 - du, v1], 0.0, 1.0);
                }
                None => batch.rect(cell, Rgba::hex(0x1E1E1E), 0.0),
            }
        }
        let xs = self.ms_to_x(&s, self.range.start_ms);
        let xe = self.ms_to_x(&s, self.range.end_ms);
        let dim = Rgba::hex(0x000000).with_alpha(0.7);
        batch.rect(Rect::new(s.x, s.y, xs - s.x, s.h), dim, 0.0);
        batch.rect(Rect::new(xe, s.y, s.x + s.w - xe, s.h), dim, 0.0);
        batch.rect(Rect::new(xs, s.y, xe - xs, 3.0 * d), accent, 0.0);
        batch.rect(Rect::new(xs, s.y + s.h - 3.0 * d, xe - xs, 3.0 * d), accent, 0.0);
        let hw = HANDLE_DP * d;
        for hx in [xs - hw, xe] {
            batch.rect(Rect::new(hx, s.y - 4.0 * d, hw, s.h + 8.0 * d), accent, 0.0);
            batch.rect(Rect::new(hx + hw / 2.0 - 1.0 * d, s.y + s.h / 2.0 - 11.0 * d, 2.0 * d, 22.0 * d), Rgba::hex(0x000000), 0.0);
        }
        // Start, length, end.
        let (x0, x1) = (s.x - hw, s.x + s.w + hw);
        let em = 14.0 * d;
        let st = time_label(self.range.start_ms);
        r.text(gl, batch, &st, x0, l.labels_y, em, 120.0 * d, Rgba::hex(QUIET), Weight::Regular);
        let en = time_label(self.range.end_ms);
        let ew = r.measure(&en, em, f.w, Weight::Regular);
        r.text(gl, batch, &en, x1 - ew, l.labels_y, em, ew + 1.0, Rgba::hex(QUIET), Weight::Regular);
        let len = length_label(self.range.len_ms(), self.range.dur_ms);
        let lem = 16.0 * d;
        let lw = r.measure(&len, lem, f.w, Weight::Regular);
        r.text(gl, batch, &len, (x0 + x1 - lw) / 2.0, l.labels_y, lem, lw + 1.0, white, Weight::Regular);

        if self.picking {
            batch.rect(Rect::new(0.0, f.h - f.ins.bottom, f.w, f.ins.bottom), Rgba::hex(0x000000), 0.0);
            self.bar = widgets::editor_bar(gl, r, batch, f.w, f.ins.top, "Choose the part", "DONE", (self.press == Some(Btn::Close), self.press == Some(Btn::Save)), true, d);
            return self.frames.iter().any(Option::is_none) && self.generation == STRIP_GEN.load(Ordering::Acquire);
        }
        // CUT: Exact / Instant, Keep sound.
        widgets::heading(gl, r, batch, x0, l.heading_y, x1 - x0, "CUT", "", d);
        for (i, (title, sub)) in MODES.iter().enumerate() {
            let row = l.rows[i];
            if self.press == Some(Btn::Mode(i)) {
                batch.rect(row, accent.with_alpha(0.3), 0.0);
            }
            let tw = row.w - 56.0 * d;
            r.text(gl, batch, title, row.x + 4.0 * d, row.y + 24.0 * d, 17.0 * d, tw, white, Weight::Regular);
            for (k, line) in widgets::wrap(r, sub, 13.0 * d, tw).iter().take(2).enumerate() {
                r.text(gl, batch, line, row.x + 4.0 * d, row.y + 44.0 * d + k as f32 * 17.0 * d, 13.0 * d, tw, Rgba::hex(SUB), Weight::Regular);
            }
            widgets::radio(batch, row.x + row.w - 20.0 * d, row.y + row.h / 2.0, self.exact == (i == 0), d);
            batch.rect(Rect::new(row.x, row.y + row.h - 1.0 * d, row.w, 1.0 * d), Rgba::hex(ROW_LINE), 0.0);
        }
        let row = l.rows[2];
        if self.press == Some(Btn::Sound) {
            batch.rect(row, accent.with_alpha(0.3), 0.0);
        }
        r.text(gl, batch, "Keep sound", row.x + 4.0 * d, widgets::baseline(r, row.y + row.h / 2.0, 17.0 * d, d), 17.0 * d, row.w - 60.0 * d, white, Weight::Regular);
        widgets::tick_box(batch, row.x + row.w - 20.0 * d, row.y + row.h / 2.0, self.keep_sound, d);
        batch.rect(Rect::new(row.x, row.y + row.h - 1.0 * d, row.w, 1.0 * d), Rgba::hex(ROW_LINE), 0.0);

        // The note: size, or what went wrong.
        let note = match &self.note {
            Some(n) => n.clone(),
            None => {
                let audio_share = (16_000.0 * self.range.dur_ms as f32 / 1000.0 / self.item.size.max(1) as f32).min(0.5);
                let bytes = estimate_bytes(self.item.size, self.range.dur_ms, self.range.len_ms(), self.keep_sound, audio_share);
                format!("Saved next to the original as a new video, about {}. The original isn't changed.", format::bytes(bytes))
            }
        };
        for (k, line) in widgets::wrap(r, &note, 14.0 * d, x1 - x0 - 8.0 * d).iter().enumerate() {
            r.text(gl, batch, line, x0 + 4.0 * d, l.note_y + k as f32 * 20.0 * d, 14.0 * d, x1 - x0, Rgba::hex(SUB), Weight::Regular);
        }

        // Cover anything below the screen's bottom edge, then the bar.
        batch.rect(Rect::new(0.0, f.h - f.ins.bottom, f.w, f.ins.bottom), Rgba::hex(0x000000), 0.0);
        let saving = self.saving.is_some();
        self.bar = widgets::editor_bar(gl, r, batch, f.w, f.ins.top, "Clip video", "SAVE COPY", (self.press == Some(Btn::Close), self.press == Some(Btn::Save)), !saving, d);

        if saving {
            // Saving: everything waits under a dim layer with the progress.
            let pct = PROGRESS.load(Ordering::Acquire).clamp(0, 100);
            batch.rect(Rect::new(0.0, 0.0, f.w, f.h), Rgba::hex(0x000000).with_alpha(0.6), 0.0);
            let (bw, bh) = (f.w - 64.0 * d, 96.0 * d);
            let b = Rect::new(32.0 * d, f.h / 2.0 - bh / 2.0, bw, bh);
            batch.rect(b, Rgba::hex(0x282828), 0.0);
            let msg = format!("Saving the clip… {pct}%");
            r.text(gl, batch, &msg, b.x + 20.0 * d, b.y + 38.0 * d, 17.0 * d, bw - 40.0 * d, white, Weight::Regular);
            let track = Rect::new(b.x + 20.0 * d, b.y + 62.0 * d, bw - 40.0 * d, 4.0 * d);
            batch.rect(track, Rgba::hex(0x555555), 0.0);
            batch.rect(Rect::new(track.x, track.y, track.w * pct as f32 / 100.0, track.h), accent, 0.0);
        }
        self.frames.iter().any(Option::is_none) && self.generation == STRIP_GEN.load(Ordering::Acquire)
    }

    fn take_frames(&mut self) {
        let mut q = STRIP.lock().unwrap();
        for (g, i, rgba) in q.drain(..) {
            if g == self.generation && i < FRAMES {
                self.frames[i] = Some(rgba);
            }
        }
    }

    fn button_at(&self, x: f32, y: f32) -> Option<Btn> {
        let f = self.last?;
        if self.bar.0.contains(x, y) {
            return Some(Btn::Close);
        }
        if self.bar.1.contains(x, y) {
            return Some(Btn::Save);
        }
        let l = self.lay(&f);
        if l.preview.contains(x, y) {
            return Some(Btn::Play);
        }
        if self.picking {
            return None;
        }
        (0..3).find(|&i| l.rows[i].contains(x, y)).map(|i| if i == 2 { Btn::Sound } else { Btn::Mode(i) })
    }

    /// Which handle a touch at `x, y` takes, if any.
    fn handle_at(&self, x: f32, y: f32) -> Option<Drag> {
        let f = self.last?;
        let s = self.lay(&f).strip;
        let d = self.d;
        if y < s.y - 16.0 * d || y > s.y + s.h + 16.0 * d {
            return None;
        }
        let hw = HANDLE_DP * d;
        let xs = self.ms_to_x(&s, self.range.start_ms) - hw / 2.0;
        let xe = self.ms_to_x(&s, self.range.end_ms) + hw / 2.0;
        let (ds, de) = ((x - xs).abs(), (x - xe).abs());
        let reach = 36.0 * d;
        match (ds <= reach, de <= reach) {
            (true, true) => Some(if ds <= de { Drag::Start } else { Drag::End }),
            (true, false) => Some(Drag::Start),
            (false, true) => Some(Drag::End),
            _ => None,
        }
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        if self.saving.is_some() {
            return Action::None;
        }
        let Some(f) = self.last else { return Action::None };
        match t.kind {
            TouchKind::Down => {
                self.drag = self.handle_at(t.x, t.y);
                if let Some(h) = self.drag {
                    let s = self.lay(&f).strip;
                    let hw = HANDLE_DP * self.d;
                    let edge = match h {
                        Drag::Start => self.ms_to_x(&s, self.range.start_ms) - hw / 2.0,
                        Drag::End => self.ms_to_x(&s, self.range.end_ms) + hw / 2.0,
                    };
                    self.grab_dx = edge - t.x;
                    self.press = None;
                    if self.playing {
                        video::pause(&self.app);
                        self.playing = false;
                    }
                } else {
                    self.press = self.button_at(t.x, t.y);
                }
            }
            TouchKind::Move => {
                if let Some(h) = self.drag {
                    let s = self.lay(&f).strip;
                    let hw = HANDLE_DP * self.d;
                    let x = t.x + self.grab_dx;
                    match h {
                        Drag::Start => {
                            self.range.set_start(self.x_to_ms(&s, x + hw / 2.0));
                            self.seek(self.range.start_ms);
                        }
                        Drag::End => {
                            self.range.set_end(self.x_to_ms(&s, x - hw / 2.0));
                            self.seek(self.range.end_ms);
                        }
                    }
                } else if self.press.is_some() && self.button_at(t.x, t.y) != self.press {
                    self.press = None;
                }
            }
            TouchKind::Up => {
                if self.drag.take().is_some() {
                    return Action::None;
                }
                let pressed = self.press.take();
                if pressed.is_some() && self.button_at(t.x, t.y) == pressed {
                    return self.pressed(pressed.unwrap());
                }
            }
            TouchKind::Cancel | TouchKind::SecondDown => {
                self.press = None;
                self.drag = None;
            }
            TouchKind::SecondUp => {}
        }
        Action::None
    }

    fn pressed(&mut self, b: Btn) -> Action {
        match b {
            Btn::Close => return Action::Close,
            Btn::Save if self.picking => {
                if self.playing {
                    video::pause(&self.app);
                    self.playing = false;
                }
                return Action::Picked(self.range);
            }
            Btn::Save => self.start_saving(),
            Btn::Play => {
                if !self.prepared {
                    return Action::None;
                }
                if self.playing {
                    video::pause(&self.app);
                    self.playing = false;
                } else {
                    if self.pos < self.range.start_ms || self.pos + 100 >= self.range.end_ms {
                        video::seek(&self.app, self.range.start_ms);
                        self.pos = self.range.start_ms;
                    }
                    self.want_seek = None;
                    video::play(&self.app);
                    self.playing = true;
                }
            }
            Btn::Mode(i) => {
                self.exact = i == 0;
                if self.note.as_deref() == Some(CANT_EXACT) {
                    self.note = None;
                }
            }
            Btn::Sound => self.keep_sound = !self.keep_sound,
        }
        Action::None
    }

    fn start_saving(&mut self) {
        if self.playing {
            video::pause(&self.app);
            self.playing = false;
        }
        self.note = None;
        PROGRESS.store(0, Ordering::Release);
        let out: Outcome = Arc::new(Mutex::new(None));
        let (app, item, range, exact, keep) = (self.app.clone(), self.item.clone(), self.range, self.exact, self.keep_sound);
        let slot = out.clone();
        let spawned = std::thread::Builder::new().name("clip-save".into()).spawn(move || {
            let t0 = Instant::now();
            let r = save(&app, &item, range, exact, keep);
            log::info!("clip: {} in {:.1} s: {r:?}", if exact { "exact" } else { "instant" }, t0.elapsed().as_secs_f64());
            *slot.lock().unwrap() = Some(r);
            holo_platform::waker::wake();
        });
        match spawned {
            Ok(_) => self.saving = Some(out),
            Err(e) => self.note = Some(format!("Couldn't save: {e}")),
        }
    }

    /// A finished save, once.
    pub fn poll(&mut self) -> Action {
        let done = self.saving.as_ref().and_then(|o| o.lock().unwrap().take());
        match done {
            None => Action::None,
            Some(r) => {
                self.saving = None;
                match r {
                    Ok(name) => Action::Saved(name),
                    Err(msg) => {
                        if msg == CANT_EXACT {
                            self.exact = false;
                        }
                        self.note = Some(msg);
                        Action::None
                    }
                }
            }
        }
    }
}

/// Write the clip into a new file next to the original. Ok(its name).
fn save(app: &AndroidApp, item: &MediaItem, range: ClipRange, exact: bool, keep_sound: bool) -> Result<String, String> {
    let copy = saving::begin(app, item.id, &item.name, true, "_clip", "mp4", "video/mp4", item.when_taken(), true).ok_or("Couldn't make the new file")?;
    let fd = copy.open_fd("rw").ok_or("Couldn't write the new file")?;
    let (start_us, end_us) = (range.start_ms as i64 * 1000, range.end_ms as i64 * 1000);
    let err = with_activity(app, |env, a| -> jni::errors::Result<Option<String>> {
        let args = [JValue::Long(item.id), JValue::Long(start_us), JValue::Long(end_us), JValue::Bool(exact), JValue::Bool(keep_sound), JValue::Int(fd)];
        let o = env.call_method(a, jni_str!("clipRun"), jni_sig!("(JJJZZI)Ljava/lang/String;"), &args)?.l()?;
        if o.is_null() {
            return Ok(None);
        }
        let s = env.cast_local::<JString>(o)?;
        Ok(Some(s.mutf8_chars(env)?.into()))
    })
    .map_err(|e| format!("Couldn't save: {e}"))?;
    if let Some(msg) = err {
        return Err(msg);
    }
    let name = copy.name.clone();
    if copy.finish() { Ok(name) } else { Err("Couldn't finish the clip".into()) }
}
