//! The Bin: what was deleted in the last 30 days, with the days each has
//! left. Tap to choose, then RESTORE or DELETE FOREVER; EMPTY BIN in ⋮.

use std::sync::Arc;
use std::time::Instant;

use gallery_core::media::{Kind, MediaItem};
use gallery_core::selection::Selection;
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
use crate::widgets::{self, MenuRow, TILE_EMPTY};

const COLS: usize = 3;
const GAP_DP: f32 = 2.0;
const BOTTOM_DP: f32 = 64.0;
const DAY_MS: i64 = 24 * 3600 * 1000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    Back,
    /// Put the chosen back (`chosen()`).
    Restore,
    /// Delete the chosen for good.
    DeleteForever,
    /// Delete everything in the Bin for good.
    Empty,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Press {
    Back,
    More,
    MenuEmpty,
    Restore,
    Delete,
}

pub struct BinScreen {
    d: f32,
    items: Arc<Vec<MediaItem>>,
    selection: Selection,
    scroller: Scroller,
    press: Option<Press>,
    menu: bool,
    menu_rows: Vec<(f32, f32)>,
    last: Option<Frame>,
}

impl BinScreen {
    pub fn new(d: f32) -> BinScreen {
        BinScreen { d, items: Arc::default(), selection: Selection::default(), scroller: Scroller::new(8.0 * d, d), press: None, menu: false, menu_rows: Vec::new(), last: None }
    }

    pub fn set_items(&mut self, items: Vec<MediaItem>) {
        let alive: std::collections::HashSet<i64> = items.iter().map(|i| i.id).collect();
        self.selection.retain(|id| alive.contains(&id));
        self.items = Arc::new(items);
    }

    pub fn items(&self) -> &[MediaItem] {
        &self.items
    }

    /// Chosen items: id, video?
    pub fn chosen(&self) -> Vec<(i64, bool)> {
        self.items.iter().filter(|i| self.selection.contains(i.id)).map(|i| (i.id, i.kind == Kind::Video)).collect()
    }

    pub fn all(&self) -> Vec<(i64, bool)> {
        self.items.iter().map(|i| (i.id, i.kind == Kind::Video)).collect()
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// Back: close the menu, then drop the choice. Returns whether it did something.
    pub fn back(&mut self) -> bool {
        if std::mem::take(&mut self.menu) {
            return true;
        }
        if self.selection.count() > 0 {
            self.selection.clear();
            return true;
        }
        false
    }

    fn top(&self, f: &Frame) -> f32 {
        f.ins.top + BAR_DP * self.d
    }

    fn bottom(&self, f: &Frame) -> f32 {
        f.h - f.ins.bottom - if self.selection.count() > 0 { BOTTOM_DP * self.d } else { 0.0 }
    }

    fn tile(&self, f: &Frame) -> f32 {
        let gap = GAP_DP * self.d;
        (f.w - f.ins.left - f.ins.right - gap * (COLS - 1) as f32) / COLS as f32
    }

    /// 0 = the help line above the grid.
    fn grid_top(&self) -> f32 {
        52.0 * self.d
    }

    fn content_h(&self, f: &Frame) -> f32 {
        let step = self.tile(f) + GAP_DP * self.d;
        self.grid_top() + self.items.len().div_ceil(COLS) as f32 * step + 24.0 * self.d
    }

    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, thumbs: &Thumbs, f: Frame, now: Instant) -> (bool, Vec<(i64, bool, u16)>) {
        let d = self.d;
        self.last = Some(f);
        let (top, bottom) = (self.top(&f), self.bottom(&f));
        self.scroller.set_bounds((self.content_h(&f) - (bottom - top)).max(0.0));
        let scroll = self.scroller.offset(now);
        let mut missing = Vec::new();
        let x0 = f.ins.left;
        let white = Rgba::hex(WHITE);

        if self.items.is_empty() {
            let em = 17.0 * d;
            for (i, msg) in ["The Bin is empty", "Deleted items stay here for 30 days"].iter().enumerate() {
                let mw = r.measure(msg, em, f.w, Weight::Light);
                r.text(gl, batch, msg, (f.w - mw) / 2.0, top + 120.0 * d + i as f32 * 28.0 * d, em, f.w, Rgba::hex(TEXT_QUIET), Weight::Light);
            }
        } else {
            let em = 14.0 * d;
            r.text(gl, batch, "Items go for good after 30 days. Tap to choose.", x0 + 16.0 * d, top + 32.0 * d - scroll, em, f.w - 32.0 * d, Rgba::hex(TEXT_QUIET), Weight::Regular);
            let tile = self.tile(&f);
            let step = tile + GAP_DP * d;
            let now_ms = crate::actions::now_ms();
            for (i, it) in self.items.iter().enumerate() {
                let y = top + self.grid_top() + (i / COLS) as f32 * step - scroll;
                if y > bottom || y + tile < top {
                    continue;
                }
                let rect = Rect::new(x0 + (i % COLS) as f32 * step, y, tile, tile);
                match r.icon_uv(&thumbs::key(it.id)) {
                    Some(uv) => batch.image(rect, uv, 0.0, 1.0),
                    None => {
                        batch.rect(rect, Rgba::hex(TILE_EMPTY), 0.0);
                        if !thumbs.failed(it.id) {
                            missing.push((it.id, it.kind == Kind::Video, it.orientation));
                        }
                    }
                }
                let on = self.selection.contains(it.id);
                if on {
                    batch.outline(rect, Rgba::hex(holo::accent()), 0.0, 3.0 * d);
                }
                widgets::checkbox(batch, rect.x + rect.w - 30.0 * d, rect.y + 4.0 * d, 26.0 * d, on);
                // Rounded up: just deleted reads "30 days", the last day "1 day".
                let days = ((it.expires_ms - now_ms + DAY_MS - 1) / DAY_MS).max(0);
                let label = match days {
                    0 => "Today".to_string(),
                    1 => "1 day".to_string(),
                    n => format!("{n} days"),
                };
                widgets::badge(gl, r, batch, rect.x + rect.w, rect.y + rect.h, &label, false, d);
            }
        }

        // Bars over the grid.
        batch.rect(Rect::new(0.0, bottom, f.w, f.h - bottom), Rgba::hex(0x000000), 0.0);
        chrome::back_bar(gl, r, batch, f.w, f.ins.top, "Bin", self.press == Some(Press::Back), d);
        let cy = f.ins.top + BAR_DP * d / 2.0;
        if !self.items.is_empty() {
            if self.press == Some(Press::More) {
                batch.rect(Rect::new(f.w - 48.0 * d, f.ins.top, 48.0 * d, BAR_DP * d), Rgba::hex(holo::accent()).with_alpha(0.3), 0.0);
            }
            widgets::overflow(batch, f.w - 26.0 * d, cy, d);
        }
        if self.selection.count() > 0 {
            let by = bottom;
            batch.rect(Rect::new(0.0, by, f.w, f.h - by), Rgba::hex(holo::BAR), 0.0);
            batch.rect(Rect::new(0.0, by, f.w, 1.0 * d), Rgba::hex(holo::DIVIDER), 0.0);
            let half = f.w / 2.0;
            for (i, (p, label, icon)) in [(Press::Restore, "RESTORE", "RESTORE"), (Press::Delete, "DELETE FOREVER", "DELETE")].into_iter().enumerate() {
                let x = half * i as f32;
                if self.press == Some(p) {
                    batch.rect(Rect::new(x, by, half, BOTTOM_DP * d), Rgba::hex(holo::accent()).with_alpha(0.3), 0.0);
                }
                widgets::action_icon(batch, icon, x + half / 2.0 - 11.0 * d, by + 12.0 * d, 22.0 * d, white);
                let lem = 12.0 * d;
                let lw = r.measure_spaced(label, lem, 0.6 * d, Weight::Medium);
                r.text_spaced(gl, batch, label, x + (half - lw) / 2.0, by + 50.0 * d, lem, 0.6 * d, half, white, Weight::Medium);
            }
        }
        if self.menu {
            let w = 200.0 * d;
            let pressed = (self.press == Some(Press::MenuEmpty)).then_some(0);
            self.menu_rows = widgets::popup(gl, r, batch, f.w - 4.0 * d - w, f.ins.top + 52.0 * d, w, &[MenuRow::Plain("Empty Bin")], pressed, d);
        }
        (self.scroller.animating(now), missing)
    }

    fn press_at(&self, x: f32, y: f32) -> Option<Press> {
        let f = self.last?;
        let d = self.d;
        if self.menu {
            let w = 200.0 * d;
            return (x >= f.w - 4.0 * d - w && self.menu_rows.iter().any(|&(ry, rh)| y >= ry && y < ry + rh)).then_some(Press::MenuEmpty);
        }
        let bar = (f.ins.top, f.ins.top + BAR_DP * d);
        if y >= bar.0 && y < bar.1 {
            if x < 160.0 * d {
                return Some(Press::Back);
            }
            if x >= f.w - 48.0 * d && !self.items.is_empty() {
                return Some(Press::More);
            }
            return None;
        }
        if self.selection.count() > 0 && y >= self.bottom(&f) && y < f.h - f.ins.bottom {
            return Some(if x < f.w / 2.0 { Press::Restore } else { Press::Delete });
        }
        None
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        let Some(f) = self.last else { return Action::None };
        let top = self.top(&f);
        match t.kind {
            TouchKind::Down => {
                self.press = self.press_at(t.x, t.y);
                if self.press.is_none() && self.menu {
                    self.menu = false;
                    return Action::None;
                }
                if self.press.is_none() && t.y >= top {
                    self.scroller.touch_down(t.x, t.y, t.t);
                }
            }
            TouchKind::Move => {
                if self.press.is_some() {
                    if self.press_at(t.x, t.y) != self.press {
                        self.press = None;
                    }
                } else {
                    self.scroller.touch_move(t.x, t.y, t.t);
                }
            }
            TouchKind::Up => {
                if let Some(p) = self.press.take() {
                    return match p {
                        Press::Back => Action::Back,
                        Press::More => {
                            self.menu = !self.menu;
                            Action::None
                        }
                        Press::MenuEmpty => {
                            self.menu = false;
                            Action::Empty
                        }
                        Press::Restore => Action::Restore,
                        Press::Delete => Action::DeleteForever,
                    };
                }
                if let ListGesture::Tap { x, y } = self.scroller.touch_up(t.x, t.y, t.t)
                    && y >= top
                {
                    let d = self.d;
                    let tile = self.tile(&f);
                    let step = tile + GAP_DP * d;
                    let cy = y - top + self.scroller.offset(t.t) - self.grid_top();
                    let cx = x - f.ins.left;
                    if cy >= 0.0 && cx >= 0.0 && cy % step < tile && cx % step < tile {
                        let i = (cy / step) as usize * COLS + (cx / step) as usize;
                        if (cx / step) < COLS as f32
                            && let Some(it) = self.items.get(i)
                        {
                            self.selection.toggle(it.id);
                        }
                    }
                }
            }
            TouchKind::Cancel => {
                self.press = None;
                self.scroller.touch_cancel(t.t);
            }
            TouchKind::SecondDown | TouchKind::SecondUp => {}
        }
        Action::None
    }
}
