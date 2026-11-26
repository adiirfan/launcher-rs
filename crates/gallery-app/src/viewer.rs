//! The full-screen viewer: swipe between pictures, pinch or double-tap to
//! zoom (with full detail decoded for the part on screen once it's still),
//! GIFs playing at their own speed, and the Holo bars with INFO.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gallery_core::dates;
use gallery_core::format;
use gallery_core::media::{Kind, MediaItem};
use gallery_core::viewer_list::index_after_removal;
use gallery_core::zoom::Zoom;
use android_activity::AndroidApp;
use glow::{Context, HasContext};
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, TEXT_QUIET, WHITE};
use holo_platform::render::Renderer;
use launcher_core::pager::{Gesture, Pager};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::chrome::BAR_DP;
use crate::gif_player::GifPlayer;
use crate::input::{Touch, TouchKind};
use crate::photo_tex::{PhotoDraw, Tex, Upload};
use crate::photos::Frame;
use crate::thumbs::{self, Thumbs};
use crate::video::{self, VideoDraw};
use crate::viewer_jobs::{Done, Job, Jobs};
use crate::widgets;

const BOTTOM_DP: f32 = 64.0;
/// The video's time and seek bar, above the bottom bar.
const SEEK_DP: f32 = 44.0;
/// The play button in the middle of a video.
const PLAY_R_DP: f32 = 36.0;
/// Full detail is decoded once the picture has been still this long.
const DETAIL_AFTER: Duration = Duration::from_millis(150);
const DOUBLE_TAP: Duration = Duration::from_millis(300);
const TOAST_FOR: Duration = Duration::from_millis(2000);
const BAR_BG: f32 = 0.6;
/// Time per frame for copying pictures to the graphics chip.
const UPLOAD_BUDGET: Duration = Duration::from_millis(4);
const LINE: u32 = 0x2A2A2A;
const PANEL: u32 = 0x141414;
const ROW_LINE: u32 = 0x262626;
const FILE_GREY: u32 = 0xBDBDBD;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    Back,
    Share(i64),
    Delete(i64),
    Favourite(i64),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Button {
    Back,
    Star,
    Share,
    Edit,
    Info,
    Delete,
    /// The middle of a video: play or pause.
    Play,
    Sound,
}

struct Pic {
    tex: Tex,
    full: (u32, u32),
}

struct Region {
    id: i64,
    rect: (u32, u32, u32, u32),
    sample: u32,
    tex: Tex,
}

enum Pending {
    Full { id: i64, full: (u32, u32), up: Upload },
    Region { id: i64, rect: (u32, u32, u32, u32), sample: u32, up: Upload },
}

impl Pending {
    fn id(&self) -> i64 {
        match self {
            Pending::Full { id, .. } | Pending::Region { id, .. } => *id,
        }
    }

    fn cancel(self, gl: &Context) {
        match self {
            Pending::Full { up, .. } | Pending::Region { up, .. } => up.cancel(gl),
        }
    }
}

struct Gif {
    id: i64,
    player: GifPlayer,
    tex: Option<Tex>,
    next_at: Instant,
}

#[derive(Clone, Copy)]
enum Mode {
    Idle,
    /// Finger down, not yet moved far enough to say what it's doing.
    Undecided { x: f32, y: f32, t: Instant },
    Pan { x: f32, y: f32 },
    Page,
    Pinch { dist: f32, mid: (f32, f32) },
    /// The rest of a gesture that already did something else.
    Ignore,
    /// Dragging along the video's seek bar.
    Seek,
}

struct Video {
    id: i64,
    tex: glow::Texture,
    prepared: bool,
    playing: bool,
    /// The newest frame's texture transform, once one has arrived.
    tm: Option<[f32; 16]>,
    pos: u64,
    dur: u64,
    /// Where the finger is on the seek bar while dragging, 0..1.
    drag: Option<f32>,
}

pub struct Viewer {
    d: f32,
    /// A picture another app handed us, outside the library: nothing to
    /// favourite or delete, so only SHARE and INFO.
    outside: bool,
    items: Arc<Vec<MediaItem>>,
    /// Indexes into `items`, in the order the grid showed them.
    order: Vec<u32>,
    pager: Pager,
    current: Option<i64>,
    pics: HashMap<i64, Pic>,
    failed: HashSet<i64>,
    region: Option<Region>,
    /// Decoded pictures still being copied to the graphics chip.
    pending: Vec<Pending>,
    zoom: Option<Zoom>,
    zoom_changed: Instant,
    gif: Option<Gif>,
    gif_bytes_failed: HashSet<i64>,
    details: Option<(i64, [String; 5])>,
    bars: bool,
    info: bool,
    toast: Option<(Instant, &'static str)>,
    mode: Mode,
    app: AndroidApp,
    video: Option<Video>,
    /// Tapped play: the video opens on the next frame (it needs the GL context).
    open_video: Option<i64>,
    video_draw: Option<VideoDraw>,
    muted: bool,
    press: Option<Button>,
    last_tap: Option<(Instant, f32, f32)>,
    pending_tap: Option<Instant>,
    last: Option<Frame>,
}

impl Viewer {
    pub fn open(app: &AndroidApp, d: f32, items: Arc<Vec<MediaItem>>, order: Vec<u32>, index: usize, width: f32) -> Viewer {
        let mut pager = Pager::new_with_slop(order.len().max(1), width.max(1.0), 8.0 * d);
        pager.jump_to(index);
        Viewer {
            d,
            outside: false,
            items,
            order,
            pager,
            current: None,
            pics: HashMap::new(),
            failed: HashSet::new(),
            region: None,
            pending: Vec::new(),
            zoom: None,
            zoom_changed: Instant::now(),
            gif: None,
            gif_bytes_failed: HashSet::new(),
            details: None,
            bars: true,
            info: false,
            toast: None,
            mode: Mode::Idle,
            app: app.clone(),
            video: None,
            open_video: None,
            video_draw: None,
            muted: false,
            press: None,
            last_tap: None,
            pending_tap: None,
            last: None,
        }
    }

    fn item(&self, page: usize) -> Option<&MediaItem> {
        self.order.get(page).map(|&i| &self.items[i as usize])
    }

    fn ids(&self) -> Vec<i64> {
        self.order.iter().map(|&i| self.items[i as usize].id).collect()
    }

    /// The library or the grid's order changed: stay on the same picture,
    /// or the next one if it's gone. Returns false when nothing is left.
    pub fn set_outside(&mut self) {
        self.outside = true;
    }

    pub fn set_list(&mut self, items: Arc<Vec<MediaItem>>, order: Vec<u32>) -> bool {
        let old = self.ids();
        let current = self.item(self.pager.page()).map(|i| i.id).unwrap_or(-1);
        self.items = items;
        self.order = order;
        let Some(at) = index_after_removal(&old, current, &self.ids()) else { return false };
        self.pager.set_pages(self.order.len());
        self.pager.jump_to(at);
        true
    }

    /// Back: close INFO first. Returns whether it did something.
    pub fn back(&mut self) -> bool {
        std::mem::take(&mut self.info)
    }

    /// Free the graphics memory; call before dropping the viewer.
    pub fn free(mut self, gl: &Context) {
        for (_, p) in self.pics.drain() {
            p.tex.free(gl);
        }
        if let Some(r) = self.region.take() {
            r.tex.free(gl);
        }
        if let Some(t) = self.gif.take().and_then(|g| g.tex) {
            t.free(gl);
        }
        for p in self.pending.drain(..) {
            p.cancel(gl);
        }
        self.close_video(gl);
    }

    fn close_video(&mut self, gl: &Context) {
        if let Some(v) = self.video.take() {
            video::close(&self.app);
            unsafe { gl.delete_texture(v.tex) };
        }
        self.open_video = None;
    }

    /// The player's news, the newest frame, and where playback is.
    fn tend_video(&mut self, gl: &Context) {
        if let Some(id) = self.open_video.take()
            && Some(id) == self.current
        {
            self.close_video(gl);
            if let Some(tex) = video::new_texture(gl) {
                video::open(&self.app, id, tex);
                self.video = Some(Video { id, tex, prepared: false, playing: false, tm: None, pos: 0, dur: 0, drag: None });
            }
        }
        let events = video::take_events();
        let Some(v) = self.video.as_mut() else { return };
        for e in events {
            match e {
                video::Event::Prepared(..) => {
                    v.prepared = true;
                    video::play(&self.app);
                    v.playing = true;
                }
                video::Event::Completed | video::Event::Paused => v.playing = false,
                video::Event::Error => {
                    self.toast = Some((Instant::now(), "Can't play this video"));
                    self.close_video(gl);
                    return;
                }
            }
        }
        if video::frame_waiting()
            && let Some(tm) = video::update(&self.app)
        {
            v.tm = Some(tm);
        }
        if v.prepared {
            let (pos, dur) = video::position(&self.app);
            v.pos = pos;
            v.dur = dur;
        }
    }

    /// Picture size as it stands upright, before the decoded one arrives.
    fn upright(it: &MediaItem) -> (u32, u32) {
        if it.orientation == 90 || it.orientation == 270 { (it.h, it.w) } else { (it.w, it.h) }
    }

    fn full_size(&self, it: &MediaItem) -> (f32, f32) {
        let (w, h) = self.pics.get(&it.id).map_or_else(|| Self::upright(it), |p| p.full);
        (w.max(1) as f32, h.max(1) as f32)
    }

    /// Take finished decodes; keep only what's near the current picture.
    fn absorb(&mut self, gl: &Context, jobs: &Jobs, now: Instant) {
        let page = self.pager.page();
        let near: HashSet<i64> = (page.saturating_sub(1)..=page + 1).filter_map(|p| self.item(p).map(|i| i.id)).collect();
        for done in jobs.take() {
            match done {
                Done::Full { id, w, h, full, rgba } if near.contains(&id) => {
                    if let Some(up) = Upload::start(gl, w, h, rgba) {
                        self.pending.push(Pending::Full { id, full, up });
                    }
                }
                Done::Region { id, rect, w, h, rgba } if self.current == Some(id) => {
                    let sample = (rect.2 / w.max(1)).max(1);
                    // A newer part replaces one still being copied.
                    for p in self.pending.extract_if(.., |p| matches!(p, Pending::Region { .. })).collect::<Vec<_>>() {
                        p.cancel(gl);
                    }
                    if let Some(up) = Upload::start(gl, w, h, rgba) {
                        self.pending.push(Pending::Region { id, rect, sample, up });
                    }
                }
                Done::Gif { id, bytes } if self.current == Some(id) => match GifPlayer::new(bytes) {
                    Some(player) => self.gif = Some(Gif { id, player, tex: None, next_at: now }),
                    None => {
                        self.gif_bytes_failed.insert(id);
                    }
                },
                Done::Details { id, fields } => self.details = Some((id, fields)),
                Done::Failed(Job::Full { id, .. }) => {
                    self.failed.insert(id);
                }
                Done::Failed(Job::Gif { id }) => {
                    self.gif_bytes_failed.insert(id);
                }
                _ => {}
            }
        }
        let stale: Vec<i64> = self.pics.keys().copied().filter(|id| !near.contains(id)).collect();
        for id in stale {
            if let Some(p) = self.pics.remove(&id) {
                p.tex.free(gl);
            }
        }
        let current = self.current;
        for p in self.pending.extract_if(.., |p| !near.contains(&p.id()) || matches!(p, Pending::Region { id, .. } if Some(*id) != current)).collect::<Vec<_>>() {
            p.cancel(gl);
        }
        self.copy_some(gl, now + UPLOAD_BUDGET);
    }

    /// Copy waiting pictures for a while: the current one first, then its
    /// full detail, then the neighbours.
    fn copy_some(&mut self, gl: &Context, until: Instant) {
        let current = self.current;
        self.pending.sort_by_key(|p| match p {
            Pending::Full { id, .. } if Some(*id) == current => 0,
            Pending::Region { .. } => 1,
            Pending::Full { .. } => 2,
        });
        while !self.pending.is_empty() && Instant::now() < until {
            match self.pending.remove(0) {
                Pending::Full { id, full, up } => match up.step(gl, until) {
                    Ok(tex) => {
                        if let Some(old) = self.pics.insert(id, Pic { tex, full }) {
                            old.tex.free(gl);
                        }
                        // Now the real size is known: fit it afresh if untouched.
                        if self.current == Some(id) && self.zoom.is_none_or(|z| z.is_fit()) {
                            self.zoom = None;
                        }
                    }
                    Err(up) => self.pending.insert(0, Pending::Full { id, full, up }),
                },
                Pending::Region { id, rect, sample, up } => match up.step(gl, until) {
                    Ok(tex) => {
                        if let Some(old) = self.region.replace(Region { id, rect, sample, tex }) {
                            old.tex.free(gl);
                        }
                    }
                    Err(up) => self.pending.insert(0, Pending::Region { id, rect, sample, up }),
                },
            }
        }
    }

    /// The page settled on another picture: drop the old one's extras.
    fn arrive(&mut self, gl: &Context) {
        let id = self.item(self.pager.page()).map(|i| i.id);
        if id == self.current {
            return;
        }
        self.current = id;
        self.zoom = None;
        if let Some(r) = self.region.take() {
            r.tex.free(gl);
        }
        if let Some(t) = self.gif.take().and_then(|g| g.tex) {
            t.free(gl);
        }
        self.close_video(gl);
    }

    /// The part on screen needs more detail than the screen-size copy has.
    fn detail_job(&self, now: Instant) -> Option<Job> {
        let id = self.current?;
        let it = self.item(self.pager.page())?;
        let (pic, z) = (self.pics.get(&id)?, self.zoom?);
        if it.kind != Kind::Image || !matches!(self.mode, Mode::Idle) || now < self.zoom_changed + DETAIL_AFTER {
            return None;
        }
        let texels_per_px = (pic.tex.w as f32 / pic.full.0.max(1) as f32) / z.scale();
        if texels_per_px >= 0.95 {
            return None;
        }
        let (x, y, w, h) = z.source_rect();
        let mut sample = 1u32;
        while (sample * 2) as f32 <= 1.0 / z.scale() {
            sample *= 2;
        }
        let rect = (x.floor() as u32, y.floor() as u32, (w.ceil() as u32).max(1), (h.ceil() as u32).max(1));
        let copying = self.pending.iter().any(|p| matches!(p, Pending::Region { rect: r, sample: s, .. } if *r == rect && *s == sample));
        if copying || self.region.as_ref().is_some_and(|r| r.rect == rect && r.sample == sample) {
            return None;
        }
        Some(Job::Region { id, orientation: it.orientation, rect, sample })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, pd: &PhotoDraw, _thumbs: &Thumbs, jobs: &Jobs, f: Frame, now: Instant) -> bool {
        let d = self.d;
        self.last = Some(f);
        self.pager.set_width(f.w);
        let screen = (f.w as i32, f.h as i32);
        self.absorb(gl, jobs, now);
        if !self.pager.animating(now) && !matches!(self.mode, Mode::Page) {
            self.arrive(gl);
        }
        let page = self.pager.page();
        self.tend_video(gl);

        // What to decode, most wanted first.
        let mut want = Vec::new();
        let max = (f.w as u32, f.h as u32);
        let full_job = |it: &MediaItem| Job::Full { id: it.id, orientation: it.orientation, max };
        let wants_full = |v: &Viewer, it: &MediaItem| {
            it.kind != Kind::Video && !v.pics.contains_key(&it.id) && !v.failed.contains(&it.id) && !v.pending.iter().any(|p| matches!(p, Pending::Full { id, .. } if *id == it.id))
        };
        if let Some(it) = self.item(page) {
            if wants_full(self, it) {
                want.push(full_job(it));
            }
            if it.kind == Kind::Gif && self.gif.as_ref().is_none_or(|g| g.id != it.id) && !self.gif_bytes_failed.contains(&it.id) {
                want.push(Job::Gif { id: it.id });
            }
            if self.info && self.details.as_ref().is_none_or(|(id, _)| *id != it.id) && it.kind != Kind::Video {
                want.push(Job::Details { id: it.id });
            }
        }
        want.extend(self.detail_job(now));
        for p in [page + 1, page.wrapping_sub(1)] {
            if let Some(it) = self.item(p)
                && wants_full(self, it)
            {
                want.push(full_job(it));
            }
        }
        jobs.want(want);

        // The next GIF frame, when it's due.
        let mut moving = self.pager.animating(now) || !matches!(self.mode, Mode::Idle) || !self.pending.is_empty();
        if let Some(g) = self.gif.as_mut() {
            moving = true;
            if now >= g.next_at
                && let Some(fr) = g.player.next()
            {
                match &g.tex {
                    Some(t) => t.update(gl, &fr.rgba),
                    None => g.tex = Tex::upload(gl, g.player.w, g.player.h, &fr.rgba),
                }
                g.next_at = now + fr.delay;
            }
        }
        if self.pending_tap.is_some_and(|t| now >= t + DOUBLE_TAP) {
            self.pending_tap = None;
            self.bars = !self.bars;
            if !self.bars {
                self.info = false;
            }
        }
        moving |= self.pending_tap.is_some() || self.toast.is_some_and(|t| now < t.0 + TOAST_FOR);
        moving |= self.open_video.is_some() || self.video.as_ref().is_some_and(|v| v.playing && v.tm.is_none());
        // Keep drawing until the zoomed picture has been still long enough
        // to ask for full detail.
        moving |= self.zoom.is_some_and(|z| !z.is_fit()) && now < self.zoom_changed + DETAIL_AFTER + Duration::from_millis(40);

        // Pictures first, straight onto the screen.
        PhotoDraw::clear(gl, screen);
        let offset = self.pager.offset(now);
        let view = (f.w, f.h);
        for p in [page.wrapping_sub(1), page, page + 1] {
            let Some(it) = self.item(p).cloned() else { continue };
            let it = &it;
            let shift = p as f32 * f.w - offset;
            if shift.abs() >= f.w {
                continue;
            }
            let full = self.full_size(it);
            let is_current = Some(it.id) == self.current;
            if is_current && self.zoom.is_none() {
                self.zoom = Some(Zoom::fit(full, view));
            }
            let z = if is_current { self.zoom.unwrap() } else { Zoom::fit(full, view) };
            let (x, y, w, h) = z.dest();
            let gif_tex = self.gif.as_ref().filter(|g| g.id == it.id).and_then(|g| g.tex.as_ref());
            if let Some(t) = gif_tex.or(self.pics.get(&it.id).map(|p| &p.tex)) {
                pd.draw(gl, t, (x + shift, y, w, h), (0.0, 0.0, 1.0, 1.0), screen);
            }
            if is_current && let Some(reg) = self.region.as_ref().filter(|r| r.id == it.id) {
                let s = z.scale();
                let (rx, ry, rw, rh) = reg.rect;
                pd.draw(gl, &reg.tex, (x + shift + rx as f32 * s, y + ry as f32 * s, rw as f32 * s, rh as f32 * s), (0.0, 0.0, 1.0, 1.0), screen);
            }
            let playing_here = self.video.as_ref().filter(|v| v.id == it.id && is_current);
            let frame_tm = playing_here.and_then(|v| v.tm.map(|tm| (v.tex, tm)));
            if let Some((tex, tm)) = frame_tm {
                let vd = self.video_draw.get_or_insert_with(|| VideoDraw::new(gl).expect("video program"));
                vd.draw(gl, tex, (x + shift, y, w, h), &tm, screen);
            } else if it.kind == Kind::Video || (!self.pics.contains_key(&it.id) && gif_tex.is_none()) {
                // Until the picture (or the video's first frame) arrives: its thumbnail.
                let side = (f.w.min(f.h) * 0.6).min(w.max(h));
                let tile = Rect::new(f.w / 2.0 - side / 2.0 + shift, f.h / 2.0 - side / 2.0, side, side);
                if let Some(uv) = r.icon_uv(&thumbs::key(it.id)) {
                    batch.image(tile, uv, 0.0, 1.0);
                }
            }
            // The play button: always on a stopped video, with the bars on a playing one.
            if it.kind == Kind::Video {
                let playing = playing_here.is_some_and(|v| v.playing);
                if !playing || (self.bars && is_current) {
                    let (cx, cy) = (f.w / 2.0 + shift, f.h / 2.0);
                    let pressed = self.press == Some(Button::Play);
                    batch.circle(cx, cy, PLAY_R_DP * d, Rgba::hex(0xFFFFFF));
                    batch.circle(cx, cy, (PLAY_R_DP - 2.0) * d, if pressed { Rgba::hex(holo::accent()).with_alpha(0.6) } else { Rgba::hex(0x000000).with_alpha(0.5) });
                    if playing {
                        for dx in [-7.0, 3.0] {
                            batch.rect(Rect::new(cx + dx * d, cy - 11.0 * d, 5.0 * d, 22.0 * d), Rgba::hex(WHITE), 0.0);
                        }
                    } else {
                        widgets::play_triangle(batch, cx - 8.0 * d, cy, 26.0 * d, Rgba::hex(WHITE));
                    }
                }
            }
        }

        if self.bars {
            self.draw_bars(gl, r, batch, &f, now);
        }
        moving
    }

    fn top_h(&self, f: &Frame) -> f32 {
        f.ins.top + BAR_DP * self.d
    }

    fn bottom_y(&self, f: &Frame) -> f32 {
        f.h - f.ins.bottom - BOTTOM_DP * self.d
    }

    fn button_rects(&self, f: &Frame) -> Vec<(Button, Rect)> {
        let d = self.d;
        let top = f.ins.top;
        let by = self.bottom_y(f);
        if self.outside {
            let q = (f.w - f.ins.left - f.ins.right) / 2.0;
            let bx = |i: f32| Rect::new(f.ins.left + q * i, by, q, BOTTOM_DP * d);
            return vec![(Button::Back, Rect::new(0.0, top, 52.0 * d, BAR_DP * d)), (Button::Share, bx(0.0)), (Button::Info, bx(1.0))];
        }
        let q = (f.w - f.ins.left - f.ins.right) / 4.0;
        let bx = |i: f32| Rect::new(f.ins.left + q * i, by, q, BOTTOM_DP * d);
        vec![
            (Button::Back, Rect::new(0.0, top, 52.0 * d, BAR_DP * d)),
            // No ⋮ menu in 0.1: nothing for it to hold yet.
            (Button::Star, Rect::new(f.w - 56.0 * d, top, 48.0 * d, BAR_DP * d)),
            (Button::Share, bx(0.0)),
            (Button::Edit, bx(1.0)),
            (Button::Info, bx(2.0)),
            (Button::Delete, bx(3.0)),
        ]
    }

    fn draw_bars(&self, gl: &Context, r: &mut Renderer, batch: &mut Batch, f: &Frame, now: Instant) {
        let d = self.d;
        let Some(it) = self.item(self.pager.page()) else { return };
        let accent = Rgba::hex(holo::accent());
        let shade = Rgba::hex(0x000000).with_alpha(BAR_BG);
        let white = Rgba::hex(WHITE);
        let pressed = |b: Button| self.press == Some(b);
        let rects = self.button_rects(f);
        let rect_of = |b: Button| rects.iter().find(|x| x.0 == b).map(|x| x.1);

        // Top: back, when and file name, star, more.
        let top_h = self.top_h(f);
        batch.rect(Rect::new(0.0, 0.0, f.w, top_h), shade, 0.0);
        for b in [Button::Back, Button::Star] {
            if let Some(rc) = rect_of(b).filter(|_| pressed(b)) {
                batch.rect(rc, accent.with_alpha(0.3), 0.0);
            }
        }
        let cy = f.ins.top + BAR_DP * d / 2.0;
        let u = d;
        icon(batch, &[&[(23.0, cy / u - 7.0), (17.0, cy / u), (23.0, cy / u + 7.0)]], 0.0, 0.0, u, 2.0 * d, white);
        let tx = 52.0 * d;
        let tw = f.w - tx - 100.0 * d;
        if it.when_taken() > 0 {
            let when = dates::short_when(it.when_taken(), f.utc_offset_s, f.today);
            r.text(gl, batch, &when, tx, cy - 2.0 * d, 17.0 * d, tw, white, Weight::Regular);
            r.text(gl, batch, &it.name, tx, cy + 17.0 * d, 13.0 * d, tw, Rgba::hex(FILE_GREY), Weight::Regular);
        } else {
            // No date known (a file from another app): just its name.
            r.text(gl, batch, &it.name, tx, widgets::baseline(r, cy, 17.0 * d, d), 17.0 * d, tw, white, Weight::Regular);
        }
        if let Some(star) = rect_of(Button::Star) {
            star_icon(batch, star.x + star.w / 2.0 - 11.0 * d, cy - 11.0 * d, 22.0 * d, it.favourite);
        }

        // Bottom: SHARE, EDIT, INFO, DELETE.
        let by = self.bottom_y(f);
        batch.rect(Rect::new(0.0, by, f.w, f.h - by), shade, 0.0);
        batch.rect(Rect::new(0.0, by, f.w, 1.0 * d), Rgba::hex(LINE), 0.0);
        for (b, label) in [(Button::Share, "SHARE"), (Button::Edit, "EDIT"), (Button::Info, "INFO"), (Button::Delete, "DELETE")] {
            let Some(rc) = rect_of(b) else { continue };
            if pressed(b) {
                batch.rect(rc, accent.with_alpha(0.3), 0.0);
            }
            let c = match b {
                Button::Info if self.info => accent,
                Button::Edit => Rgba::hex(0x6E6E6E),
                _ => white,
            };
            let (ix, iy, s) = (rc.x + rc.w / 2.0 - 11.0 * d, rc.y + 12.0 * d, 22.0 * d);
            bottom_icon(batch, b, ix, iy, s, it.kind == Kind::Video, c);
            let em = 12.0 * d;
            let lw = r.measure_spaced(label, em, 0.6 * d, Weight::Medium);
            r.text_spaced(gl, batch, label, rc.x + (rc.w - lw) / 2.0, rc.y + 50.0 * d, em, 0.6 * d, rc.w, c, Weight::Medium);
        }

        let mut above = by;
        if it.kind == Kind::Video {
            above = by - SEEK_DP * d;
            self.draw_seek(gl, r, batch, f, it, above);
        }
        if self.info {
            self.draw_info(gl, r, batch, f, it, above);
        }
        if let Some((t, msg)) = self.toast.filter(|t| now < t.0 + TOAST_FOR) {
            let _ = t;
            let by = above;
            let em = 14.0 * d;
            let mw = r.measure(msg, em, f.w, Weight::Regular);
            let (w, h) = (mw + 40.0 * d, 40.0 * d);
            let rc = Rect::new((f.w - w) / 2.0, by - h - 24.0 * d, w, h);
            batch.rect(rc, Rgba::hex(0x333333).with_alpha(0.95), 4.0 * d);
            r.text(gl, batch, msg, rc.x + 20.0 * d, widgets::baseline(r, rc.y + h / 2.0, em, d), em, mw + 1.0, white, Weight::Regular);
        }
    }

    fn seek_rect(&self, f: &Frame) -> Rect {
        let d = self.d;
        Rect::new(0.0, self.bottom_y(f) - SEEK_DP * d, f.w, SEEK_DP * d)
    }

    /// The bar's track: x from, x to.
    fn seek_track(&self, f: &Frame) -> (f32, f32) {
        let d = self.d;
        (f.ins.left + 16.0 * d + 48.0 * d, f.w - f.ins.right - 16.0 * d - 32.0 * d - 48.0 * d)
    }

    /// "0:12 ━━━●──── 1:04 🔊", as the mockup.
    fn draw_seek(&self, gl: &Context, r: &mut Renderer, batch: &mut Batch, f: &Frame, it: &MediaItem, y: f32) {
        let d = self.d;
        let accent = Rgba::hex(holo::accent());
        let white = Rgba::hex(WHITE);
        let rc = self.seek_rect(f);
        batch.rect(rc, Rgba::hex(0x000000).with_alpha(BAR_BG), 0.0);
        let v = self.video.as_ref().filter(|v| v.id == it.id);
        let dur = v.map(|v| v.dur).filter(|&d| d > 0).unwrap_or(it.duration_ms);
        let frac = match v {
            Some(v) => v.drag.unwrap_or(if dur > 0 { v.pos as f32 / dur as f32 } else { 0.0 }),
            None => 0.0,
        }
        .clamp(0.0, 1.0);
        let pos = (frac * dur as f32) as u64;
        let cy = y + SEEK_DP * d / 2.0;
        let em = 13.0 * d;
        let base = widgets::baseline(r, cy, em, d);
        r.text(gl, batch, &format::duration(pos), f.ins.left + 16.0 * d, base, em, 48.0 * d, white, Weight::Regular);
        let (x0, x1) = self.seek_track(f);
        batch.rect(Rect::new(x0, cy - 1.5 * d, x1 - x0, 3.0 * d), Rgba::hex(0x555555), 0.0);
        let xp = x0 + (x1 - x0) * frac;
        batch.rect(Rect::new(x0, cy - 1.5 * d, xp - x0, 3.0 * d), accent, 0.0);
        batch.circle(xp, cy, 7.5 * d, accent);
        r.text(gl, batch, &format::duration(dur), x1 + 12.0 * d, base, em, 48.0 * d, white, Weight::Regular);
        // Sound: a speaker, with waves unless muted.
        let (sx, sy, u) = (f.w - f.ins.right - 16.0 * d - 20.0 * d, cy - 10.0 * d, 20.0 * d / 24.0);
        let lw = 1.8 * u;
        icon(batch, &[&[(4.0, 9.0), (8.0, 9.0), (13.0, 5.0), (13.0, 19.0), (8.0, 15.0), (4.0, 15.0), (4.0, 9.0)]], sx, sy, u, lw, white);
        if self.muted {
            icon(batch, &[&[(16.0, 9.0), (22.0, 15.0)], &[(22.0, 9.0), (16.0, 15.0)]], sx, sy, u, lw, white);
        } else {
            icon(batch, &[&[(16.0, 9.0), (17.5, 12.0), (16.0, 15.0)], &[(18.5, 6.5), (21.0, 12.0), (18.5, 17.5)]], sx, sy, u, lw, white);
        }
        if self.press == Some(Button::Sound) {
            batch.rect(Rect::new(f.w - f.ins.right - 56.0 * d, y, 56.0 * d, SEEK_DP * d), accent.with_alpha(0.3), 0.0);
        }
    }

    pub fn is_video(&self) -> bool {
        self.item(self.pager.page()).is_some_and(|i| i.kind == Kind::Video)
    }

    fn info_rows(&self, f: &Frame, it: &MediaItem) -> Vec<(&'static str, String)> {
        let full = self.pics.get(&it.id).map_or_else(|| Self::upright(it), |p| p.full);
        let mut size = format!("{} · {} × {}", format::bytes(it.size), full.0, full.1);
        if it.kind == Kind::Video {
            size.push_str(&format!(" · {}", format::duration(it.duration_ms)));
        }
        let mut rows = vec![("Taken", dates::full_when(it.when_taken(), f.utc_offset_s)), ("File", it.name.clone()), ("Size", size)];
        if it.when_taken() <= 0 {
            rows.remove(0);
        }
        if let Some((id, fields)) = &self.details
            && *id == it.id
        {
            let camera: Vec<&str> = fields[..4].iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect();
            if !camera.is_empty() {
                rows.push(("Camera", camera.join(" · ")));
            }
            if !fields[4].is_empty() {
                rows.push(("Location", fields[4].replace(',', ", ")));
            }
        }
        if !it.folder.is_empty() {
            rows.push(("Folder", it.folder.clone()));
        }
        rows
    }

    fn draw_info(&self, gl: &Context, r: &mut Renderer, batch: &mut Batch, f: &Frame, it: &MediaItem, bottom: f32) {
        let d = self.d;
        let rows = self.info_rows(f, it);
        let row_h = 58.0 * d;
        let h = 4.0 * d + 40.0 * d + rows.len() as f32 * row_h + 12.0 * d;
        let y = bottom - h;
        let accent = Rgba::hex(holo::accent());
        batch.rect(Rect::new(0.0, y, f.w, h), Rgba::hex(PANEL), 0.0);
        batch.rect(Rect::new(0.0, y, f.w, 2.0 * d), accent, 0.0);
        let x = f.ins.left + 16.0 * d;
        let w = f.w - f.ins.left - f.ins.right - 32.0 * d;
        widgets::heading(gl, r, batch, x, y + 16.0 * d, w, "DETAILS", "", d);
        let mut ry = y + 44.0 * d;
        for (k, v) in rows {
            r.text(gl, batch, k, x + 4.0 * d, ry + 22.0 * d, 13.0 * d, w, Rgba::hex(TEXT_QUIET), Weight::Regular);
            r.text(gl, batch, &v, x + 4.0 * d, ry + 43.0 * d, 16.0 * d, w - 8.0 * d, Rgba::hex(WHITE), Weight::Regular);
            batch.rect(Rect::new(x, ry + row_h - 1.0 * d, w, 1.0 * d), Rgba::hex(ROW_LINE), 0.0);
            ry += row_h;
        }
    }

    fn button_at(&self, x: f32, y: f32) -> Option<Button> {
        let f = self.last?;
        if self.is_video() {
            let playing = self.video.as_ref().is_some_and(|v| v.playing);
            let r = PLAY_R_DP * self.d;
            if (!playing || self.bars) && (x - f.w / 2.0).hypot(y - f.h / 2.0) <= r * 1.3 {
                return Some(Button::Play);
            }
            if self.bars && self.seek_rect(&f).contains(x, y) && x >= f.w - f.ins.right - 56.0 * self.d {
                return Some(Button::Sound);
            }
        }
        if !self.bars {
            return None;
        }
        self.button_rects(&f).iter().find(|(_, rc)| rc.contains(x, y)).map(|(b, _)| *b)
    }

    fn in_bars(&self, y: f32) -> bool {
        let Some(f) = self.last else { return false };
        let bottom = if self.is_video() { self.bottom_y(&f) - SEEK_DP * self.d } else { self.bottom_y(&f) };
        self.bars && (y < self.top_h(&f) || y >= bottom)
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        let d = self.d;
        match t.kind {
            TouchKind::Down => {
                self.press = self.button_at(t.x, t.y);
                if let (Some(f), None) = (self.last, self.press)
                    && self.bars
                    && self.is_video()
                    && self.seek_rect(&f).contains(t.x, t.y)
                    && let (x0, x1) = self.seek_track(&f)
                    && let Some(v) = self.video.as_mut().filter(|v| v.prepared)
                {
                    v.drag = Some(((t.x - x0) / (x1 - x0)).clamp(0.0, 1.0));
                    self.mode = Mode::Seek;
                    return Action::None;
                }
                self.mode = if self.press.is_some() || self.in_bars(t.y) { Mode::Ignore } else { Mode::Undecided { x: t.x, y: t.y, t: t.t } };
            }
            TouchKind::Move => match self.mode {
                Mode::Undecided { x, y, t: t0 } => {
                    let (dx, dy) = (t.x - x, t.y - y);
                    if dx.hypot(dy) < 8.0 * d {
                        return Action::None;
                    }
                    let zoomed = self.zoom.is_some_and(|z| !z.is_fit());
                    let sideways = dx.abs() > dy.abs();
                    let stuck = zoomed && sideways && {
                        let mut probe = self.zoom.unwrap();
                        probe.pan(dx, 0.0) == dx
                    };
                    if zoomed && !stuck {
                        if let Some(z) = self.zoom.as_mut() {
                            z.pan(dx, dy);
                        }
                        self.zoom_changed = t.t;
                        self.mode = Mode::Pan { x: t.x, y: t.y };
                    } else {
                        self.pager.touch_down(x, y, t0);
                        self.pager.touch_move(t.x, t.y, t.t);
                        self.mode = Mode::Page;
                    }
                }
                Mode::Pan { x, y } => {
                    if let Some(z) = self.zoom.as_mut() {
                        z.pan(t.x - x, t.y - y);
                    }
                    self.zoom_changed = t.t;
                    self.mode = Mode::Pan { x: t.x, y: t.y };
                }
                Mode::Page => self.pager.touch_move(t.x, t.y, t.t),
                Mode::Pinch { dist, mid } => {
                    if let Some(p) = t.second {
                        let nd = (t.x - p.0).hypot(t.y - p.1).max(1.0);
                        let nm = ((t.x + p.0) / 2.0, (t.y + p.1) / 2.0);
                        if let Some(z) = self.zoom.as_mut() {
                            z.pinch(nd / dist, nm);
                            z.pan(nm.0 - mid.0, nm.1 - mid.1);
                        }
                        self.zoom_changed = t.t;
                        self.mode = Mode::Pinch { dist: nd, mid: nm };
                    }
                }
                Mode::Seek => {
                    if let Some(f) = self.last {
                        let (x0, x1) = self.seek_track(&f);
                        if let Some(v) = self.video.as_mut() {
                            v.drag = Some(((t.x - x0) / (x1 - x0)).clamp(0.0, 1.0));
                        }
                    }
                }
                Mode::Idle | Mode::Ignore => {
                    if self.press.is_some() && self.button_at(t.x, t.y) != self.press {
                        self.press = None;
                    }
                }
            },
            TouchKind::SecondDown => {
                if let Some(p) = t.second
                    && !matches!(self.mode, Mode::Ignore)
                {
                    if matches!(self.mode, Mode::Page) {
                        self.pager.touch_cancel(t.t);
                    }
                    self.pending_tap = None;
                    self.mode = Mode::Pinch { dist: (t.x - p.0).hypot(t.y - p.1).max(1.0), mid: ((t.x + p.0) / 2.0, (t.y + p.1) / 2.0) };
                }
            }
            TouchKind::SecondUp => {
                if matches!(self.mode, Mode::Pinch { .. }) {
                    self.mode = Mode::Ignore;
                }
            }
            TouchKind::Up => {
                let mode = std::mem::replace(&mut self.mode, Mode::Idle);
                if let Mode::Seek = mode
                    && let Some(v) = self.video.as_mut()
                    && let Some(frac) = v.drag.take()
                {
                    let ms = (frac * v.dur as f32) as u64;
                    v.pos = ms;
                    video::seek(&self.app, ms);
                    return Action::None;
                }
                if let Some(b) = self.press.take() {
                    return self.pressed(b, t.t);
                }
                match mode {
                    Mode::Page => {
                        if let Gesture::Tap { .. } = self.pager.touch_up(t.x, t.y, t.t) {
                            self.tap(t.x, t.y, t.t);
                        }
                    }
                    Mode::Undecided { .. } => self.tap(t.x, t.y, t.t),
                    _ => {}
                }
            }
            TouchKind::Cancel => {
                if matches!(self.mode, Mode::Page) {
                    self.pager.touch_cancel(t.t);
                }
                self.mode = Mode::Idle;
                self.press = None;
            }
        }
        Action::None
    }

    /// One tap waits a moment in case a second makes it a double tap.
    fn tap(&mut self, x: f32, y: f32, t: Instant) {
        let near = self.last_tap.is_some_and(|(t0, x0, y0)| t.duration_since(t0) < DOUBLE_TAP && (x - x0).hypot(y - y0) < 40.0 * self.d);
        if near {
            self.last_tap = None;
            self.pending_tap = None;
            if let Some(z) = self.zoom.as_mut() {
                z.double_tap((x, y));
                self.zoom_changed = t;
            }
        } else {
            self.last_tap = Some((t, x, y));
            self.pending_tap = Some(t);
        }
    }

    fn pressed(&mut self, b: Button, t: Instant) -> Action {
        let Some(id) = self.item(self.pager.page()).map(|i| i.id) else { return Action::None };
        match b {
            Button::Back => Action::Back,
            Button::Star => Action::Favourite(id),
            Button::Share => Action::Share(id),
            Button::Delete => Action::Delete(id),
            Button::Info => {
                self.info = !self.info;
                Action::None
            }
            Button::Edit => {
                self.toast = Some((t, "Coming in a later version"));
                Action::None
            }
            Button::Play => {
                match self.video.as_mut().filter(|v| v.id == id) {
                    Some(v) if v.playing => {
                        video::pause(&self.app);
                        v.playing = false;
                    }
                    Some(v) if v.prepared => {
                        // At the end: from the start again.
                        if v.dur > 0 && v.pos + 250 >= v.dur {
                            video::seek(&self.app, 0);
                        }
                        video::play(&self.app);
                        v.playing = true;
                    }
                    Some(_) => {}
                    None => self.open_video = Some(id),
                }
                Action::None
            }
            Button::Sound => {
                self.muted = !self.muted;
                video::mute(&self.app, self.muted);
                Action::None
            }
        }
    }
}

/// Lines through points given on a `u`-sized grid, offset by `x, y`.
fn icon(batch: &mut Batch, lines: &[&[(f32, f32)]], x: f32, y: f32, u: f32, width: f32, c: Rgba) {
    for pts in lines {
        for w in pts.windows(2) {
            holo::stroke(batch, (x + w[0].0 * u, y + w[0].1 * u), (x + w[1].0 * u, y + w[1].1 * u), width, c);
        }
    }
}

const STAR: [(f32, f32); 11] = [
    (12.0, 3.0),
    (14.7, 8.6),
    (20.8, 9.5),
    (16.4, 13.8),
    (17.4, 19.9),
    (12.0, 17.0),
    (6.6, 19.9),
    (7.6, 13.8),
    (3.2, 9.5),
    (9.3, 8.6),
    (12.0, 3.0),
];

/// The favourite star: an outline, filled with the accent when chosen.
fn star_icon(batch: &mut Batch, x: f32, y: f32, size: f32, on: bool) {
    let u = size / 24.0;
    let c = if on { Rgba::hex(holo::accent()) } else { Rgba::hex(WHITE) };
    if on {
        // Fill: strokes from the middle out to points all round the edge.
        let mid = (12.0, 12.6);
        for w in STAR.windows(2) {
            for k in 0..8 {
                let f = k as f32 / 8.0;
                let p = (w[0].0 + (w[1].0 - w[0].0) * f, w[0].1 + (w[1].1 - w[0].1) * f);
                holo::stroke(batch, (x + mid.0 * u, y + mid.1 * u), (x + p.0 * u, y + p.1 * u), 1.6 * u, c);
            }
        }
    }
    icon(batch, &[&STAR], x, y, u, 1.8 * u, c);
}

fn circle_ring(batch: &mut Batch, cx: f32, cy: f32, r: f32, width: f32, c: Rgba) {
    batch.outline(Rect::new(cx - r, cy - r, r * 2.0, r * 2.0), c, r, width);
}

/// The bottom bar's icons, on a 24-unit square as the mockup's.
fn bottom_icon(batch: &mut Batch, b: Button, x: f32, y: f32, size: f32, video: bool, c: Rgba) {
    let u = size / 24.0;
    let lw = 1.8 * u;
    match b {
        Button::Share => widgets::action_icon(batch, "SHARE", x, y, size, c),
        Button::Edit if video => {
            circle_ring(batch, x + 6.0 * u, y + 6.0 * u, 2.0 * u, lw, c);
            circle_ring(batch, x + 6.0 * u, y + 18.0 * u, 2.0 * u, lw, c);
            icon(batch, &[&[(7.5, 7.5), (20.0, 18.0)], &[(7.5, 16.5), (20.0, 6.0)]], x, y, u, lw, c);
        }
        Button::Edit => icon(batch, &[&[(4.0, 20.0), (8.0, 20.0), (19.0, 9.0), (15.0, 5.0), (4.0, 16.0), (4.0, 20.0)], &[(13.0, 7.0), (17.0, 11.0)]], x, y, u, lw, c),
        Button::Info => {
            circle_ring(batch, x + 12.0 * u, y + 12.0 * u, 9.0 * u, lw, c);
            icon(batch, &[&[(12.0, 11.0), (12.0, 17.0)], &[(12.0, 7.3), (12.0, 8.3)]], x, y, u, lw, c);
        }
        Button::Delete => widgets::action_icon(batch, "DELETE", x, y, size, c),
        _ => {}
    }
}
