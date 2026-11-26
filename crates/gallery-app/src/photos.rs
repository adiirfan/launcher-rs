//! The Photos screen (and an album, which is the same grid with a filter):
//! day, letter or size headings over a grid of thumbnails, the sort menu,
//! pinch for bigger or smaller tiles, and a fast-scroll handle.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gallery_core::access::Access;
use gallery_core::arrange::{arrange, Arranged, Filter, Order, SortKey};
use gallery_core::dates::CivilDate;
use gallery_core::format;
use gallery_core::grid::{default_cols, lay_out, pinch_cols, Grid, GridSpec, Row};
use gallery_core::media::{Kind, MediaItem};
use glow::Context;
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, BLACK, TEXT_QUIET, WHITE};
use holo_platform::render::Renderer;
use launcher_core::geometry::Insets;
use launcher_core::scroll::{ListGesture, Scroller};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::chrome::{self, BAR_DP};
use crate::input::{Touch, TouchKind};
use crate::thumbs::{self, Thumbs};
use crate::widgets::{self, MenuRow, TABS_DP, TILE_EMPTY};

const HEADER_DP: f32 = 50.0;
const GAP_DP: f32 = 2.0;
const BOTTOM_PAD_DP: f32 = 24.0;
/// The fast-scroll handle shows while scrolling and this long after.
const FAST_LINGER: Duration = Duration::from_millis(1500);
const FAST_W_DP: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    /// A tile was tapped: its index in `arranged().order`.
    Open(u32),
    Albums,
    Back,
    Select,
    Bin,
    Settings,
    AskAccess,
}

#[derive(Clone, Copy, PartialEq)]
enum Menu {
    None,
    Sort,
    More,
}

#[derive(Clone, Copy, PartialEq)]
enum Press {
    Tab(usize),
    Spinner,
    Overflow,
    Back,
    MenuRow(usize),
    Button,
}

struct Pinch {
    dist: f32,
    anchor: Option<u32>,
}

/// What the screen knows about the bars and the day, set every frame.
#[derive(Clone, Copy)]
pub struct Frame {
    pub w: f32,
    pub h: f32,
    pub ins: Insets,
    pub today: CivilDate,
    pub utc_offset_s: i32,
}

pub struct Photos {
    pub filter: Filter,
    /// An album's name (with a back arrow); `None` for the main screen with tabs.
    title: Option<String>,
    d: f32,
    items: Arc<Vec<MediaItem>>,
    arranged: Arranged,
    grid: Grid,
    spec: GridSpec,
    /// Columns chosen by pinching; `None` follows the screen width.
    pinched: Option<u32>,
    pub sort: SortKey,
    pub order: Order,
    pub hidden: HashSet<i64>,
    scroller: Scroller,
    dirty: bool,
    laid_for: Option<(f32, CivilDate, i32)>,
    menu: Menu,
    menu_x: f32,
    menu_w: f32,
    menu_rows: Vec<(f32, f32)>,
    press: Option<Press>,
    press_at: (f32, f32),
    pinch: Option<Pinch>,
    /// A two-finger gesture happened: the rest of it neither scrolls nor taps.
    eaten: bool,
    fast_until: Option<Instant>,
    fast_drag: bool,
    last: Option<Frame>,
    pub glow: bool,
}

impl Photos {
    pub fn new(d: f32, filter: Filter, title: Option<String>) -> Photos {
        Photos {
            filter,
            title,
            d,
            items: Arc::default(),
            arranged: Arranged::default(),
            grid: Grid::default(),
            spec: GridSpec { width_px: 1.0, density: d, cols: 4, gap_px: GAP_DP * d, header_px: HEADER_DP * d },
            pinched: None,
            sort: SortKey::Taken,
            order: Order::Desc,
            hidden: HashSet::new(),
            scroller: Scroller::new(8.0 * d, d),
            dirty: true,
            laid_for: None,
            menu: Menu::None,
            menu_x: 0.0,
            menu_w: 0.0,
            menu_rows: Vec::new(),
            press: None,
            press_at: (0.0, 0.0),
            pinch: None,
            eaten: false,
            fast_until: None,
            fast_drag: false,
            last: None,
            glow: true,
        }
    }

    pub fn set_items(&mut self, items: Arc<Vec<MediaItem>>) {
        self.items = items;
        self.dirty = true;
    }

    pub fn items(&self) -> &Arc<Vec<MediaItem>> {
        &self.items
    }

    pub fn arranged(&self) -> &Arranged {
        &self.arranged
    }

    pub fn set_density(&mut self, d: f32) {
        if d != self.d {
            self.d = d;
            self.scroller = Scroller::new(8.0 * d, d);
            self.dirty = true;
        }
    }

    pub fn set_sort(&mut self, sort: SortKey, order: Order) {
        if (sort, order) != (self.sort, self.order) {
            self.sort = sort;
            self.order = order;
            self.dirty = true;
            self.scroller.jump_to(0.0);
        }
    }

    pub fn set_hidden(&mut self, hidden: HashSet<i64>) {
        if hidden != self.hidden {
            self.hidden = hidden;
            self.dirty = true;
        }
    }

    pub fn columns(&self) -> u32 {
        self.spec.cols
    }

    fn top(&self, f: &Frame) -> f32 {
        f.ins.top + BAR_DP * self.d + if self.title.is_none() { TABS_DP * self.d } else { 0.0 }
    }

    fn bottom(&self, f: &Frame) -> f32 {
        f.h - f.ins.bottom
    }

    fn view_h(&self, f: &Frame) -> f32 {
        self.bottom(f) - self.top(f)
    }

    fn max_scroll(&self, f: &Frame) -> f32 {
        (self.grid.height + BOTTOM_PAD_DP * self.d - self.view_h(f)).max(0.0)
    }

    /// Sort and lay out again when the list, sort, width or day changed,
    /// keeping the tile row at the top of the view in place.
    fn relayout(&mut self, f: &Frame, now: Instant) {
        let key = (f.w, f.today, f.utc_offset_s);
        let cols = self.pinched.unwrap_or_else(|| default_cols(f.w / self.d));
        if !self.dirty && self.laid_for == Some(key) && cols == self.spec.cols {
            return;
        }
        let same_list = !self.dirty;
        let anchor = self.grid.anchor(self.scroller.offset(now));
        if self.dirty {
            self.arranged = arrange(&self.items, self.sort, self.order, &self.filter, f.today, f.utc_offset_s, &self.hidden);
        }
        self.spec = GridSpec { width_px: f.w - f.ins.left - f.ins.right, density: self.d, cols, gap_px: GAP_DP * self.d, header_px: HEADER_DP * self.d };
        self.grid = lay_out(&self.arranged, &self.spec);
        self.scroller.set_bounds(self.max_scroll(f));
        if same_list && let Some(a) = anchor {
            self.scroller.jump_to(self.grid.scroll_for(a));
        }
        self.dirty = false;
        self.laid_for = Some(key);
    }

    /// Draws the screen; returns whether something is moving, and the
    /// thumbnails it is missing (on screen first).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, thumbs: &Thumbs, f: Frame, access: Access, now: Instant) -> (bool, Vec<(i64, bool, u16)>) {
        let d = self.d;
        self.last = Some(f);
        self.relayout(&f, now);
        let (top, bottom) = (self.top(&f), self.bottom(&f));
        let mut missing = Vec::new();
        let mut moving = self.scroller.animating(now) || self.pinch.is_some();
        let x0 = f.ins.left;

        if access != Access::Full && self.items.is_empty() || access == Access::None {
            self.draw_no_access(gl, r, batch, &f, access, top);
        } else if self.arranged.order.is_empty() {
            let em = 17.0 * d;
            let msg = if self.title.is_some() { "Nothing in this album" } else { "No photos or videos yet" };
            let mw = r.measure(msg, em, f.w, Weight::Light);
            r.text(gl, batch, msg, (f.w - mw) / 2.0, top + 120.0 * d, em, f.w, Rgba::hex(TEXT_QUIET), Weight::Light);
        } else {
            let scroll = self.scroller.offset(now);
            let view = bottom - top;
            let rows = self.grid.visible(scroll, view);
            let step = self.grid.tile_px + self.spec.gap_px;
            for i in rows.clone() {
                let row = self.grid.rows[i];
                let sy = top + row.y() - scroll;
                match row {
                    Row::Header { group, .. } => {
                        let g = &self.arranged.groups[group as usize];
                        widgets::heading(gl, r, batch, x0 + 12.0 * d, sy + 18.0 * d, f.w - f.ins.left - f.ins.right - 24.0 * d, &g.label, &format::count(g.len as usize), d);
                    }
                    Row::Tiles { first, count, .. } => {
                        for k in 0..count {
                            let item = &self.items[self.arranged.order[(first + k) as usize] as usize];
                            let rect = Rect::new(x0 + k as f32 * step, sy, self.grid.tile_px, self.grid.tile_px);
                            match r.icon_uv(&thumbs::key(item.id)) {
                                Some(uv) => batch.image(rect, uv, 0.0, 1.0),
                                None => {
                                    batch.rect(rect, Rgba::hex(TILE_EMPTY), 0.0);
                                    if thumbs.failed(item.id) {
                                        let s = 24.0 * d;
                                        chrome::picture_glyph(batch, rect.x + (rect.w - s) / 2.0, rect.y + (rect.h - s) / 2.0, s, Rgba::hex(0x5A5A5A));
                                    } else {
                                        missing.push((item.id, item.kind == Kind::Video, item.orientation));
                                    }
                                }
                            }
                            match item.kind {
                                Kind::Video => widgets::badge(gl, r, batch, rect.x + rect.w, rect.y + rect.h, &format::duration(item.duration_ms), true, d),
                                Kind::Gif => widgets::badge(gl, r, batch, rect.x + rect.w, rect.y + rect.h, "GIF", false, d),
                                Kind::Image => {}
                            }
                        }
                    }
                }
            }
            // Load a little ahead in both directions, after what's on screen.
            let ahead = self.grid.visible((scroll - view * 0.5).max(0.0), view * 2.0);
            for i in ahead {
                if rows.contains(&i) {
                    continue;
                }
                if let Row::Tiles { first, count, .. } = self.grid.rows[i] {
                    for k in 0..count {
                        let item = &self.items[self.arranged.order[(first + k) as usize] as usize];
                        if r.icon_uv(&thumbs::key(item.id)).is_none() && !thumbs.failed(item.id) {
                            missing.push((item.id, item.kind == Kind::Video, item.orientation));
                        }
                    }
                }
            }

            // The Holo glow at whichever end is being pushed past.
            let over = self.scroller.overscroll();
            if self.glow && over != 0.0 {
                let strength = (over.abs() / (160.0 * d)).min(1.0);
                let y = if over < 0.0 { top } else { bottom };
                batch.glow(Rect::new(f.w * 0.15, y, f.w * 0.7, 1.0), Rgba::hex(holo::accent()).with_alpha(0.55 * strength), 0.0, 28.0 * d * (0.5 + strength));
            }

            // Fast scroll: only for lists several screens long.
            if self.grid.height > view * 3.0 {
                if self.scroller.animating(now) || self.scroller.moved() || self.fast_drag {
                    self.fast_until = Some(now + FAST_LINGER);
                }
                if self.fast_until.is_some_and(|t| now < t) {
                    moving = true;
                    let max = self.max_scroll(&f).max(1.0);
                    let th = 48.0 * d;
                    let ty = top + (scroll / max).clamp(0.0, 1.0) * (view - th);
                    let tw = if self.fast_drag { 10.0 * d } else { 6.0 * d };
                    let right = f.w - f.ins.right;
                    batch.rect(Rect::new(right - tw - 2.0 * d, ty, tw, th), Rgba::hex(holo::accent()), 0.0);
                    if self.fast_drag
                        && let Some(label) = self.grid.label_at(&self.arranged, scroll)
                    {
                        let em = 16.0 * d;
                        let lw = r.measure_spaced(label, em, 0.6 * d, Weight::Bold);
                        let (bw, bh) = (lw + 32.0 * d, 48.0 * d);
                        let bx = right - 24.0 * d - bw;
                        let by = (ty + th / 2.0 - bh / 2.0).clamp(top, bottom - bh);
                        batch.rect(Rect::new(bx, by, bw, bh), Rgba::hex(0x222222), 0.0);
                        batch.outline(Rect::new(bx, by, bw, bh), Rgba::hex(holo::accent()), 0.0, 1.0 * d);
                        r.text_spaced(gl, batch, label, bx + 16.0 * d, widgets::baseline(r, by + bh / 2.0, em, d), em, 0.6 * d, lw + 1.0, Rgba::hex(holo::accent()), Weight::Bold);
                    }
                }
            }
        }

        // Bars on top of whatever scrolled behind them.
        batch.rect(Rect::new(0.0, bottom, f.w, f.h - bottom), Rgba::hex(BLACK), 0.0);
        let bar_top = f.ins.top;
        match &self.title {
            None => {
                chrome::action_bar(gl, r, batch, f.w, bar_top, "Gallery", d);
                widgets::tabs(gl, r, batch, 0.0, bar_top + BAR_DP * d, f.w, ["PHOTOS", "ALBUMS"], 0, d);
            }
            Some(t) => {
                let t = t.clone();
                chrome::back_bar(gl, r, batch, f.w, bar_top, &t, self.press == Some(Press::Back), d);
            }
        }
        let cy = bar_top + BAR_DP * d / 2.0;
        if self.press == Some(Press::Overflow) {
            batch.rect(Rect::new(f.w - 48.0 * d, bar_top, 48.0 * d, BAR_DP * d), Rgba::hex(holo::accent()).with_alpha(0.3), 0.0);
        }
        widgets::overflow(batch, f.w - 26.0 * d, cy, d);
        if access == Access::Full {
            let names = [(SortKey::Taken, "DATE TAKEN"), (SortKey::Changed, "DATE CHANGED"), (SortKey::Name, "NAME"), (SortKey::Size, "SIZE")];
            let value = names.iter().find(|n| n.0 == self.sort).map_or("", |n| n.1);
            if self.press == Some(Press::Spinner) {
                batch.rect(Rect::new(f.w - 48.0 * d - 140.0 * d, bar_top, 140.0 * d, BAR_DP * d), Rgba::hex(holo::accent()).with_alpha(0.3), 0.0);
            }
            widgets::spinner(gl, r, batch, f.w - 48.0 * d, cy, "SORT", value, d);
        }

        // Menus over everything.
        let menu_top = bar_top + 52.0 * d;
        let pressed_row = match self.press {
            Some(Press::MenuRow(i)) => Some(i),
            _ => None,
        };
        match self.menu {
            Menu::None => self.menu_rows.clear(),
            Menu::Sort => {
                let rows = self.sort_rows();
                let rows: Vec<MenuRow> = rows.iter().map(|(row, _)| row.as_menu_row()).collect();
                self.menu_w = 236.0 * d;
                self.menu_x = f.w - 48.0 * d - self.menu_w;
                self.menu_rows = widgets::popup(gl, r, batch, self.menu_x, menu_top, self.menu_w, &rows, pressed_row, d);
            }
            Menu::More => {
                let rows = [MenuRow::Plain("Select"), MenuRow::Plain("Bin"), MenuRow::Plain("Settings")];
                self.menu_w = 200.0 * d;
                self.menu_x = f.w - 4.0 * d - self.menu_w;
                self.menu_rows = widgets::popup(gl, r, batch, self.menu_x, menu_top, self.menu_w, &rows, pressed_row, d);
            }
        }
        (moving, missing)
    }

    fn draw_no_access(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, f: &Frame, access: Access, top: f32) {
        let d = self.d;
        let x = f.ins.left + 16.0 * d;
        let w = f.w - f.ins.left - f.ins.right - 32.0 * d;
        widgets::heading(gl, r, batch, x, top + 24.0 * d, w, "YOUR PHOTOS AND VIDEOS", "", d);
        let lines: &[&str] = if access == Access::Limited {
            &["Gallery can only see the pictures you", "picked. Allow all photos and videos", "so it can show your whole library."]
        } else {
            &["Gallery needs to see the photos and", "videos on this phone to show them.", "Nothing leaves the phone."]
        };
        let em = 17.0 * d;
        for (i, line) in lines.iter().enumerate() {
            r.text(gl, batch, line, x + 4.0 * d, top + 90.0 * d + i as f32 * 26.0 * d, em, w, Rgba::hex(WHITE), Weight::Light);
        }
        let b = self.button_rect(f, top);
        widgets::button(gl, r, batch, b, "ALLOW", self.press == Some(Press::Button), d);
    }

    fn button_rect(&self, f: &Frame, top: f32) -> Rect {
        let d = self.d;
        Rect::new(f.ins.left + 16.0 * d, top + 180.0 * d, 160.0 * d, 48.0 * d)
    }

    fn sort_rows(&self) -> Vec<(OwnedRow, Option<SortChoice>)> {
        let mut v = vec![(OwnedRow::Caption("SORT BY"), None)];
        for (k, name) in [(SortKey::Taken, "Date taken"), (SortKey::Changed, "Date changed"), (SortKey::Name, "Name"), (SortKey::Size, "Size")] {
            v.push((OwnedRow::Radio(name, self.sort == k), Some(SortChoice::Key(k))));
        }
        v.push((OwnedRow::Caption("ORDER"), None));
        let (desc, asc) = match self.sort {
            SortKey::Name => ("Z to A", "A to Z"),
            SortKey::Size => ("Largest first", "Smallest first"),
            _ => ("Newest first", "Oldest first"),
        };
        v.push((OwnedRow::Radio(desc, self.order == Order::Desc), Some(SortChoice::Order(Order::Desc))));
        v.push((OwnedRow::Radio(asc, self.order == Order::Asc), Some(SortChoice::Order(Order::Asc))));
        v
    }

    fn menu_row_at(&self, x: f32, y: f32) -> Option<usize> {
        if x < self.menu_x || x > self.menu_x + self.menu_w {
            return None;
        }
        self.menu_rows.iter().position(|&(ry, rh)| y >= ry && y < ry + rh)
    }

    fn press_at_point(&self, x: f32, y: f32, f: &Frame, access: Access) -> Option<Press> {
        let d = self.d;
        if self.menu != Menu::None {
            return self.menu_row_at(x, y).map(Press::MenuRow);
        }
        let bar_top = f.ins.top;
        let bar_bottom = bar_top + BAR_DP * d;
        if y >= bar_top && y < bar_bottom {
            if x >= f.w - 48.0 * d {
                return Some(Press::Overflow);
            }
            if access == Access::Full && x >= f.w - 48.0 * d - 140.0 * d {
                return Some(Press::Spinner);
            }
            if self.title.is_some() && x < 160.0 * d {
                return Some(Press::Back);
            }
            return None;
        }
        if self.title.is_none() && y >= bar_bottom && y < bar_bottom + TABS_DP * d {
            return Some(Press::Tab(if x < f.w / 2.0 { 0 } else { 1 }));
        }
        let showing_access = access == Access::None || (access != Access::Full && self.items.is_empty());
        if showing_access {
            let b = self.button_rect(f, self.top(f));
            if b.contains(x, y) {
                return Some(Press::Button);
            }
        }
        None
    }

    pub fn touch(&mut self, t: Touch, access: Access) -> Action {
        let Some(f) = self.last else { return Action::None };
        let d = self.d;
        let top = self.top(&f);
        match t.kind {
            TouchKind::Down => {
                self.eaten = false;
                self.press_at = (t.x, t.y);
                if self.menu != Menu::None {
                    self.press = self.press_at_point(t.x, t.y, &f, access);
                    if self.press.is_none() {
                        // A tap outside closes the menu and does nothing else.
                        self.menu = Menu::None;
                        self.eaten = true;
                    }
                    return Action::None;
                }
                self.press = self.press_at_point(t.x, t.y, &f, access);
                if self.press.is_some() {
                    return Action::None;
                }
                let fast_visible = self.fast_until.is_some_and(|u| t.t < u);
                if fast_visible && t.x > f.w - f.ins.right - FAST_W_DP * d && t.y >= top {
                    self.fast_drag = true;
                    self.fast_jump(t.y, &f);
                    return Action::None;
                }
                if t.y >= top {
                    self.scroller.touch_down(t.x, t.y, t.t);
                }
            }
            TouchKind::SecondDown => {
                if let Some(p) = t.second.filter(|_| self.press.is_none() && !self.fast_drag) {
                    self.scroller.touch_cancel(t.t);
                    self.eaten = true;
                    let anchor = self.grid.anchor(self.scroller.offset(t.t));
                    self.pinch = Some(Pinch { dist: (t.x - p.0).hypot(t.y - p.1).max(1.0), anchor });
                }
            }
            TouchKind::Move => {
                if let (Some(p), Some(pinch)) = (t.second, self.pinch.as_mut()) {
                    let dist = (t.x - p.0).hypot(t.y - p.1).max(1.0);
                    let cols = pinch_cols(self.spec.cols, dist / pinch.dist, f.w / d);
                    if cols != self.spec.cols {
                        pinch.dist = dist;
                        let anchor = pinch.anchor;
                        self.pinched = Some(cols);
                        self.relayout(&f, t.t);
                        if let Some(a) = anchor {
                            self.scroller.jump_to(self.grid.scroll_for(a));
                        }
                    }
                    return Action::None;
                }
                if self.eaten {
                    return Action::None;
                }
                if self.fast_drag {
                    self.fast_jump(t.y, &f);
                    return Action::None;
                }
                if self.press.is_some() {
                    if (t.x - self.press_at.0).hypot(t.y - self.press_at.1) > 12.0 * d {
                        self.press = None;
                    }
                    return Action::None;
                }
                self.scroller.touch_move(t.x, t.y, t.t);
            }
            TouchKind::SecondUp => {
                self.pinch = None;
            }
            TouchKind::Up => {
                self.pinch = None;
                if std::mem::take(&mut self.eaten) {
                    self.press = None;
                    self.scroller.touch_cancel(t.t);
                    return Action::None;
                }
                if std::mem::take(&mut self.fast_drag) {
                    self.fast_until = Some(t.t + FAST_LINGER);
                    return Action::None;
                }
                if let Some(p) = self.press.take() {
                    return self.pressed(p);
                }
                if let ListGesture::Tap { x, y } = self.scroller.touch_up(t.x, t.y, t.t)
                    && y >= top
                {
                    let cy = y - top + self.scroller.offset(t.t);
                    if let Some(i) = self.grid.item_at(x - f.ins.left, cy, &self.spec) {
                        return Action::Open(i);
                    }
                }
            }
            TouchKind::Cancel => {
                self.pinch = None;
                self.press = None;
                self.eaten = false;
                self.fast_drag = false;
                self.scroller.touch_cancel(t.t);
            }
        }
        Action::None
    }

    fn fast_jump(&mut self, y: f32, f: &Frame) {
        let top = self.top(f);
        let th = 48.0 * self.d;
        let frac = ((y - top - th / 2.0) / (self.view_h(f) - th)).clamp(0.0, 1.0);
        self.scroller.jump_to(frac * self.max_scroll(f));
    }

    fn pressed(&mut self, p: Press) -> Action {
        match p {
            Press::Tab(1) => Action::Albums,
            Press::Tab(_) => Action::None,
            Press::Back => Action::Back,
            Press::Spinner => {
                self.menu = if self.menu == Menu::Sort { Menu::None } else { Menu::Sort };
                Action::None
            }
            Press::Overflow => {
                self.menu = if self.menu == Menu::More { Menu::None } else { Menu::More };
                Action::None
            }
            Press::Button => Action::AskAccess,
            Press::MenuRow(i) => match self.menu {
                Menu::Sort => {
                    match self.sort_rows().get(i).and_then(|r| r.1) {
                        Some(SortChoice::Key(k)) => {
                            let order = if k == SortKey::Name { Order::Asc } else { Order::Desc };
                            self.set_sort(k, order);
                        }
                        Some(SortChoice::Order(o)) => self.set_sort(self.sort, o),
                        None => return Action::None,
                    }
                    self.menu = Menu::None;
                    Action::None
                }
                Menu::More => {
                    self.menu = Menu::None;
                    match i {
                        0 => Action::Select,
                        1 => Action::Bin,
                        _ => Action::Settings,
                    }
                }
                Menu::None => Action::None,
            },
        }
    }

    /// Sort and lay out now if anything changed (the viewer needs the
    /// order while the grid itself isn't being drawn).
    pub fn refresh(&mut self, f: &Frame) {
        self.relayout(f, Instant::now());
    }

    /// Back: close a menu if one is open. Returns whether it did something.
    pub fn back(&mut self) -> bool {
        if self.menu != Menu::None {
            self.menu = Menu::None;
            return true;
        }
        false
    }

    /// The id of the item at `index` in display order.
    pub fn item_at_order(&self, index: u32) -> Option<&MediaItem> {
        self.arranged.order.get(index as usize).map(|&i| &self.items[i as usize])
    }
}

#[derive(Clone, Copy)]
enum SortChoice {
    Key(SortKey),
    Order(Order),
}

enum OwnedRow {
    Caption(&'static str),
    Radio(&'static str, bool),
}

impl OwnedRow {
    fn as_menu_row(&self) -> MenuRow<'static> {
        match *self {
            OwnedRow::Caption(s) => MenuRow::Caption(s),
            OwnedRow::Radio(s, on) => MenuRow::Radio(s, on),
        }
    }
}
