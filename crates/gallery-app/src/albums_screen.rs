//! The Albums tab: two columns of album covers under "ON THIS PHONE", then
//! a MORE list (Bin, Hidden albums, Settings), as the approved mockup.

use std::collections::HashSet;
use std::time::Instant;

use gallery_core::albums::{albums, Album};
use gallery_core::format;
use gallery_core::media::{Kind, MediaItem};
use glow::Context;
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, TEXT_QUIET, WHITE};
use holo_platform::render::Renderer;
use launcher_core::scroll::{ListGesture, Scroller};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::chrome::{self, BAR_DP};
use crate::input::{Touch, TouchKind};
use crate::photos::Frame;
use crate::thumbs::{self, Thumbs};
use crate::widgets::{self, TABS_DP, TILE_EMPTY};

const PAD_DP: f32 = 12.0;
const COL_GAP_DP: f32 = 12.0;
const ROW_GAP_DP: f32 = 16.0;
/// Under a cover: gap, name line, count line.
const CAPTION_DP: f32 = 46.0;
const MORE_ROW_DP: f32 = 64.0;
const ROW_LINE: u32 = 0x262626;
const ICON: u32 = 0xBDBDBD;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Photos,
    Open(Album),
    Bin,
    Hidden,
    Settings,
}

const MORE: [(&str, &str); 3] = [
    ("Bin", "Deleted items stay here for 30 days"),
    ("Hidden albums", "Folders you chose to keep out of Photos"),
    ("Settings", "Grid size, accent colour, updates"),
];

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    PhotosTab,
    Album(usize),
    More(usize),
}

/// Where everything sits, in content pixels (0 = just under the tabs).
struct Layout {
    cell: f32,
    grid_top: f32,
    more_top: f32,
    height: f32,
}

pub struct AlbumsScreen {
    d: f32,
    list: Vec<Album>,
    items: std::sync::Arc<Vec<MediaItem>>,
    hidden: HashSet<i64>,
    dirty: bool,
    scroller: Scroller,
    press: Option<Hit>,
    last: Option<Frame>,
}

impl AlbumsScreen {
    pub fn new(d: f32) -> AlbumsScreen {
        AlbumsScreen {
            d,
            list: Vec::new(),
            items: Default::default(),
            hidden: HashSet::new(),
            dirty: true,
            scroller: Scroller::new(8.0 * d, d),
            press: None,
            last: None,
        }
    }

    pub fn set_items(&mut self, items: std::sync::Arc<Vec<MediaItem>>) {
        self.items = items;
        self.dirty = true;
    }

    pub fn set_hidden(&mut self, hidden: HashSet<i64>) {
        if hidden != self.hidden {
            self.hidden = hidden;
            self.dirty = true;
        }
    }

    pub fn set_density(&mut self, d: f32) {
        if d != self.d {
            self.d = d;
            self.scroller = Scroller::new(8.0 * d, d);
        }
    }

    fn top(&self, f: &Frame) -> f32 {
        f.ins.top + (BAR_DP + TABS_DP) * self.d
    }

    fn layout(&self, f: &Frame) -> Layout {
        let d = self.d;
        let inner = f.w - f.ins.left - f.ins.right - 2.0 * PAD_DP * d;
        let cell = (inner - COL_GAP_DP * d) / 2.0;
        let grid_top = 54.0 * d;
        let rows = self.list.len().div_ceil(2) as f32;
        let grid_h = if rows > 0.0 { rows * (cell + CAPTION_DP * d) + (rows - 1.0) * ROW_GAP_DP * d } else { 0.0 };
        let more_top = grid_top + grid_h + 28.0 * d;
        let height = more_top + 24.0 * d + MORE.len() as f32 * MORE_ROW_DP * d + 24.0 * d;
        Layout { cell, grid_top, more_top, height }
    }

    fn cell_rect(&self, f: &Frame, l: &Layout, i: usize) -> Rect {
        let d = self.d;
        let x = f.ins.left + PAD_DP * d + (i % 2) as f32 * (l.cell + COL_GAP_DP * d);
        let y = l.grid_top + (i / 2) as f32 * (l.cell + CAPTION_DP * d + ROW_GAP_DP * d);
        Rect::new(x, y, l.cell, l.cell + CAPTION_DP * d)
    }

    fn more_rect(&self, f: &Frame, l: &Layout, i: usize) -> Rect {
        let d = self.d;
        let x = f.ins.left + PAD_DP * d;
        Rect::new(x, l.more_top + 24.0 * d + i as f32 * MORE_ROW_DP * d, f.w - f.ins.right - PAD_DP * d - x, MORE_ROW_DP * d)
    }

    /// What's under screen point `x, y`.
    fn hit(&self, x: f32, y: f32, now: Instant) -> Option<Hit> {
        let f = self.last?;
        let bar_bottom = f.ins.top + BAR_DP * self.d;
        let top = self.top(&f);
        if y >= bar_bottom && y < top {
            return (x < f.w / 2.0).then_some(Hit::PhotosTab);
        }
        if y < top || y >= f.h - f.ins.bottom {
            return None;
        }
        let cy = y - top + self.scroller.offset(now);
        let l = self.layout(&f);
        if let Some(i) = (0..self.list.len()).find(|&i| self.cell_rect(&f, &l, i).contains(x, cy)) {
            return Some(Hit::Album(i));
        }
        (0..MORE.len()).find(|&i| self.more_rect(&f, &l, i).contains(x, cy)).map(Hit::More)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, thumbs: &Thumbs, f: Frame, now: Instant) -> (bool, Vec<(i64, bool, u16)>) {
        let d = self.d;
        self.last = Some(f);
        if self.dirty {
            self.list = albums(&self.items, &self.hidden);
            self.dirty = false;
        }
        let l = self.layout(&f);
        let top = self.top(&f);
        let bottom = f.h - f.ins.bottom;
        self.scroller.set_bounds((l.height - (bottom - top)).max(0.0));
        let scroll = self.scroller.offset(now);
        let oy = top - scroll;
        let x0 = f.ins.left + PAD_DP * d;
        let wide = f.w - f.ins.right - PAD_DP * d - x0;
        let mut missing = Vec::new();

        widgets::heading(gl, r, batch, x0, oy + 18.0 * d, wide, "ON THIS PHONE", "", d);
        for i in 0..self.list.len() {
            let c = self.cell_rect(&f, &l, i);
            let (x, y) = (c.x, c.y + oy);
            if y > bottom || y + c.h < top {
                continue;
            }
            let a = &self.list[i];
            let cover = Rect::new(x, y, l.cell, l.cell);
            batch.glow(Rect::new(x, y + 2.0 * d, l.cell, l.cell), Rgba::hex(0x000000).with_alpha(0.7), 0.0, 4.0 * d);
            match r.icon_uv(&thumbs::key(a.cover)) {
                Some(uv) => batch.image(cover, uv, 0.0, 1.0),
                None => {
                    batch.rect(cover, Rgba::hex(TILE_EMPTY), 0.0);
                    if !thumbs.failed(a.cover)
                        && let Some(it) = self.items.iter().find(|it| it.id == a.cover)
                    {
                        missing.push((it.id, it.kind == Kind::Video, it.orientation));
                    }
                }
            }
            if self.press == Some(Hit::Album(i)) {
                batch.rect(cover, Rgba::hex(holo::accent()).with_alpha(0.35), 0.0);
            }
            batch.rect(Rect::new(x, y + l.cell - 3.0 * d, l.cell, 3.0 * d), Rgba::hex(holo::accent()), 0.0);
            r.text(gl, batch, &a.name, x, y + l.cell + 22.0 * d, 16.0 * d, l.cell, Rgba::hex(WHITE), Weight::Regular);
            r.text(gl, batch, &format::count(a.count as usize), x, y + l.cell + 40.0 * d, 13.0 * d, l.cell, Rgba::hex(TEXT_QUIET), Weight::Regular);
        }

        widgets::heading(gl, r, batch, x0, oy + l.more_top, wide, "MORE", "", d);
        for (i, (name, sub)) in MORE.iter().enumerate() {
            let m = self.more_rect(&f, &l, i);
            let y = m.y + oy;
            if self.press == Some(Hit::More(i)) {
                batch.rect(Rect::new(m.x, y, m.w, m.h), Rgba::hex(holo::accent()).with_alpha(0.3), 0.0);
            }
            more_icon(batch, i, m.x + 4.0 * d, y + m.h / 2.0 - 12.0 * d, 24.0 * d);
            let tx = m.x + 44.0 * d;
            r.text(gl, batch, name, tx, y + 28.0 * d, 17.0 * d, m.w - 48.0 * d, Rgba::hex(WHITE), Weight::Regular);
            r.text(gl, batch, sub, tx, y + 48.0 * d, 13.0 * d, m.w - 48.0 * d, Rgba::hex(TEXT_QUIET), Weight::Regular);
            batch.rect(Rect::new(m.x, y + m.h - 1.0 * d, m.w, 1.0 * d), Rgba::hex(ROW_LINE), 0.0);
        }

        // The bars over whatever scrolled behind them.
        batch.rect(Rect::new(0.0, bottom, f.w, f.h - bottom), Rgba::hex(0x000000), 0.0);
        chrome::action_bar(gl, r, batch, f.w, f.ins.top, "Gallery", d);
        widgets::tabs(gl, r, batch, 0.0, f.ins.top + BAR_DP * d, f.w, ["PHOTOS", "ALBUMS"], 1, d);
        if self.press == Some(Hit::PhotosTab) {
            batch.rect(Rect::new(0.0, f.ins.top + BAR_DP * d, f.w / 2.0, TABS_DP * d), Rgba::hex(holo::accent()).with_alpha(0.3), 0.0);
        }
        (self.scroller.animating(now), missing)
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        let Some(f) = self.last else { return Action::None };
        match t.kind {
            TouchKind::Down => {
                self.press = self.hit(t.x, t.y, t.t);
                if t.y >= self.top(&f) {
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
                let hit = self.press.take();
                let tap = matches!(self.scroller.touch_up(t.x, t.y, t.t), ListGesture::Tap { .. });
                return match hit {
                    Some(Hit::PhotosTab) => Action::Photos,
                    Some(Hit::Album(i)) if tap => self.list.get(i).cloned().map_or(Action::None, Action::Open),
                    Some(Hit::More(0)) if tap => Action::Bin,
                    Some(Hit::More(1)) if tap => Action::Hidden,
                    Some(Hit::More(_)) if tap => Action::Settings,
                    _ => Action::None,
                };
            }
            TouchKind::Cancel | TouchKind::SecondDown => {
                self.press = None;
                self.scroller.touch_cancel(t.t);
            }
            TouchKind::SecondUp => {}
        }
        Action::None
    }
}

/// The MORE rows' line icons, drawn on a 24-unit square as the mockup's.
fn more_icon(batch: &mut Batch, which: usize, x: f32, y: f32, size: f32) {
    let u = size / 24.0;
    let c = Rgba::hex(ICON);
    let lw = 1.8 * u;
    let p = |a: f32, b: f32| (x + a * u, y + b * u);
    let line = |batch: &mut Batch, pts: &[(f32, f32)]| {
        for w in pts.windows(2) {
            holo::stroke(batch, p(w[0].0, w[0].1), p(w[1].0, w[1].1), lw, c);
        }
    };
    match which {
        0 => {
            line(batch, &[(5.0, 7.0), (19.0, 7.0)]);
            line(batch, &[(9.0, 7.0), (9.0, 4.0), (15.0, 4.0), (15.0, 7.0)]);
            line(batch, &[(7.0, 7.0), (8.0, 20.0), (16.0, 20.0), (17.0, 7.0)]);
        }
        1 => {
            line(batch, &[(3.0, 12.0), (7.0, 7.5), (12.0, 6.0), (17.0, 7.5), (21.0, 12.0), (17.0, 16.5), (12.0, 18.0), (7.0, 16.5), (3.0, 12.0)]);
            line(batch, &[(4.0, 4.0), (20.0, 20.0)]);
        }
        _ => {
            batch.outline(Rect::new(x + 9.0 * u, y + 9.0 * u, 6.0 * u, 6.0 * u), c, 3.0 * u, lw);
            for (a, b, e, f) in [(12.0, 2.0, 12.0, 5.0), (12.0, 19.0, 12.0, 22.0), (2.0, 12.0, 5.0, 12.0), (19.0, 12.0, 22.0, 12.0), (4.9, 4.9, 7.0, 7.0), (17.0, 17.0, 19.1, 19.1), (4.9, 19.1, 7.0, 17.0), (17.0, 7.0, 19.1, 4.9)] {
                line(batch, &[(a, b), (e, f)]);
            }
        }
    }
}
