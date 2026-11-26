//! The gallery's settings: looks like the launcher's Holo settings (accent
//! section headings, rows with a grey line under them, tick boxes), with a
//! second page for choosing hidden albums.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use gallery_core::albums::{albums, Album};
use gallery_core::arrange::Filter;
use gallery_core::media::{Kind, MediaItem};
use gallery_core::settings::{GallerySettings, GRID_SIZES};
use glow::Context;
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, ROW_LINE, WHITE};
use holo_platform::render::Renderer;
use holo_platform::update;
use launcher_core::scroll::{ListGesture, Scroller};
use launcher_core::settings::Accent;
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::chrome::{self, BAR_DP};
use crate::input::{Touch, TouchKind};
use crate::photos::Frame;
use crate::thumbs::{self, Thumbs};
use crate::widgets::{self, MenuRow, TILE_EMPTY};

const SUB: u32 = 0x9E9E9E;
const BOX_FILL: u32 = 0x1A1A1A;
const BOX_EDGE: u32 = 0x8C8C8C;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    Back,
    /// The settings changed: save and apply them.
    Changed,
    /// Open Android's "Media management" page.
    ManageMedia,
    /// Check for updates (or install what a check found).
    Update,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RowId {
    Accent,
    Grid,
    Hidden,
    Manage,
    Internet,
    Updates,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Page {
    Main,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Picker {
    Accent,
    Grid,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Back,
    Row(RowId),
    Album(usize),
    Choice(usize),
    Outside,
}

enum Block {
    Section { title: &'static str, y: f32 },
    Row { id: RowId, y: f32, h: f32, sub: Vec<String> },
}

fn grid_name(cols: u32) -> String {
    if cols == 0 { "Automatic".into() } else { format!("{cols} across") }
}

/// A row's title, its line underneath, and its tick box (if it has one).
fn row_text(id: RowId, s: &GallerySettings, manage: Option<bool>) -> (&'static str, String, Option<bool>) {
    match id {
        RowId::Accent => ("Accent colour", s.accent.name().into(), None),
        RowId::Grid => ("Grid size", format!("{}. Pinch the grid to change it for now", grid_name(s.cols)), None),
        RowId::Hidden => (
            "Hidden albums",
            match s.hidden.len() {
                0 => "Choose folders to keep out of Photos and Albums".into(),
                1 => "1 folder hidden".into(),
                n => format!("{n} folders hidden"),
            },
            None,
        ),
        RowId::Manage => (
            "Media management",
            if manage == Some(true) {
                "Allowed. Deleting and restoring go ahead without Android's confirm box".into()
            } else {
                "Not allowed. Android asks to confirm each delete and restore. Tap to allow".into()
            },
            None,
        ),
        RowId::Internet => ("Internet access", "Only when you tap Check for updates, to ask GitHub for a newer version. Nothing else goes online".into(), None),
        RowId::Updates => ("Check for updates", update::line("Gallery"), None),
    }
}

pub struct SettingsScreen {
    d: f32,
    page: Page,
    scroller: Scroller,
    pressed: Option<Hit>,
    picker: Option<Picker>,
    picker_rows: Vec<(f32, f32)>,
    picker_x: (f32, f32),
    blocks: Vec<Block>,
    laid_out: Option<(GallerySettings, f32, Option<bool>, u32)>,
    /// Every folder on the phone, hidden ones included.
    folders: Vec<Album>,
    /// For the folders' cover pictures: kind and orientation by id.
    covers: HashMap<i64, (bool, u16)>,
    last: Option<Frame>,
}

impl SettingsScreen {
    pub fn new(d: f32, items: &Arc<Vec<MediaItem>>) -> SettingsScreen {
        let mut s = SettingsScreen {
            d,
            page: Page::Main,
            scroller: Scroller::new(8.0 * d, d),
            pressed: None,
            picker: None,
            picker_rows: Vec::new(),
            picker_x: (0.0, 0.0),
            blocks: Vec::new(),
            laid_out: None,
            folders: Vec::new(),
            covers: HashMap::new(),
            last: None,
        };
        s.set_items(items);
        s
    }

    pub fn set_items(&mut self, items: &Arc<Vec<MediaItem>>) {
        self.folders = albums(items, &HashSet::new()).into_iter().filter(|a| matches!(a.filter, Filter::Bucket(_))).collect();
        let wanted: HashSet<i64> = self.folders.iter().map(|a| a.cover).collect();
        self.covers = items.iter().filter(|i| wanted.contains(&i.id)).map(|i| (i.id, (i.kind == Kind::Video, i.orientation))).collect();
    }

    /// Start on the hidden albums page.
    pub fn open_hidden(&mut self) {
        self.page = Page::Hidden;
    }

    /// Back: close the menu, then the hidden albums page. False: leave.
    pub fn back(&mut self) -> bool {
        if self.picker.take().is_some() {
            return true;
        }
        if self.page == Page::Hidden {
            self.page = Page::Main;
            self.scroller.reset();
            return true;
        }
        false
    }

    fn list_top(&self, f: &Frame) -> f32 {
        f.ins.top + BAR_DP * self.d
    }

    fn list_bottom(&self, f: &Frame) -> f32 {
        f.h - f.ins.bottom
    }

    fn album_row_h(&self) -> f32 {
        64.0 * self.d
    }

    fn sections(manage: Option<bool>) -> Vec<(&'static str, Vec<RowId>)> {
        let mut out = vec![("LOOK", vec![RowId::Accent, RowId::Grid]), ("ALBUMS", vec![RowId::Hidden])];
        if manage.is_some() {
            out.push(("PERMISSIONS", vec![RowId::Manage]));
        }
        out.push(("PRIVACY", vec![RowId::Internet]));
        out.push(("ABOUT", vec![RowId::Updates]));
        out
    }

    fn wrap(r: &mut Renderer, s: &str, em: f32, max: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in s.split(' ') {
            let longer = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && r.measure(&longer, em, f32::MAX, Weight::Regular) > max {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = longer;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    fn lay_out(&mut self, r: &mut Renderer, s: &GallerySettings, manage: Option<bool>, w: f32) {
        let key = (s.clone(), w, manage, update::generation());
        if self.laid_out.as_ref() == Some(&key) {
            return;
        }
        let d = self.d;
        let mut blocks = Vec::new();
        let mut y = 0.0;
        for (title, rows) in Self::sections(manage) {
            blocks.push(Block::Section { title, y });
            y += 41.0 * d;
            for id in rows {
                let (_, sub, tick) = row_text(id, s, manage);
                let max = w - 40.0 * d - if tick.is_some() { 60.0 * d } else { 0.0 };
                let sub = Self::wrap(r, &sub, 14.0 * d, max);
                let h = (51.0 + 19.0 * sub.len() as f32).max(72.0) * d;
                blocks.push(Block::Row { id, y, h, sub });
                y += h;
            }
        }
        self.blocks = blocks;
        self.laid_out = Some(key);
    }

    fn content_h(&self) -> f32 {
        match self.page {
            Page::Main => self.blocks.last().map_or(0.0, |b| match b {
                Block::Row { y, h, .. } => y + h,
                Block::Section { y, .. } => *y,
            }) + 40.0 * self.d,
            Page::Hidden => self.folders.len() as f32 * self.album_row_h() + 24.0 * self.d,
        }
    }

    fn choices(&self, s: &GallerySettings) -> Vec<(String, bool)> {
        match self.picker {
            Some(Picker::Accent) => Accent::ALL.iter().map(|a| (a.name().to_string(), *a == s.accent)).collect(),
            Some(Picker::Grid) => GRID_SIZES.iter().map(|c| (grid_name(*c), *c == s.cols)).collect(),
            None => Vec::new(),
        }
    }

    fn hit(&self, x: f32, y: f32, now: Instant) -> Hit {
        let Some(f) = self.last else { return Hit::Outside };
        let d = self.d;
        if self.picker.is_some() {
            let (px, pw) = self.picker_x;
            return match self.picker_rows.iter().position(|&(ry, rh)| x >= px && x < px + pw && y >= ry && y < ry + rh) {
                Some(i) => Hit::Choice(i),
                None => Hit::Outside,
            };
        }
        if y >= f.ins.top && y < self.list_top(&f) {
            return if x < 160.0 * d { Hit::Back } else { Hit::Outside };
        }
        if y < self.list_top(&f) || y >= self.list_bottom(&f) {
            return Hit::Outside;
        }
        let ly = y - self.list_top(&f) + self.scroller.offset(now);
        match self.page {
            Page::Main => self
                .blocks
                .iter()
                .find_map(|b| match b {
                    Block::Row { id, y, h, .. } if ly >= *y && ly < y + h => Some(Hit::Row(*id)),
                    _ => None,
                })
                .unwrap_or(Hit::Outside),
            Page::Hidden => {
                let i = (ly / self.album_row_h()) as usize;
                if ly >= 0.0 && i < self.folders.len() { Hit::Album(i) } else { Hit::Outside }
            }
        }
    }

    pub fn touch(&mut self, t: Touch, s: &mut GallerySettings) -> Action {
        match t.kind {
            TouchKind::Down => {
                self.pressed = Some(self.hit(t.x, t.y, t.t));
                if self.picker.is_none() {
                    self.scroller.touch_down(t.x, t.y, t.t);
                }
            }
            TouchKind::Move => {
                if self.picker.is_none() {
                    self.scroller.touch_move(t.x, t.y, t.t);
                    if self.scroller.moved() {
                        self.pressed = None;
                    }
                } else if self.pressed.is_some() && self.pressed != Some(self.hit(t.x, t.y, t.t)) {
                    self.pressed = None;
                }
            }
            TouchKind::Cancel => {
                self.scroller.touch_cancel(t.t);
                self.pressed = None;
            }
            TouchKind::Up => {
                let pressed = self.pressed.take();
                let tap = self.picker.is_some() || matches!(self.scroller.touch_up(t.x, t.y, t.t), ListGesture::Tap { .. });
                let now_hit = self.hit(t.x, t.y, t.t);
                if tap && pressed == Some(now_hit) {
                    return self.act(now_hit, s);
                }
            }
            TouchKind::SecondDown | TouchKind::SecondUp => {}
        }
        Action::None
    }

    fn act(&mut self, hit: Hit, s: &mut GallerySettings) -> Action {
        match hit {
            Hit::Back => {
                if !self.back() {
                    return Action::Back;
                }
            }
            Hit::Outside => self.picker = None,
            Hit::Choice(i) => {
                match self.picker.take() {
                    Some(Picker::Accent) => s.accent = Accent::ALL[i.min(Accent::ALL.len() - 1)],
                    Some(Picker::Grid) => s.cols = GRID_SIZES[i.min(GRID_SIZES.len() - 1)],
                    None => {}
                }
                return Action::Changed;
            }
            Hit::Album(i) => {
                if let Some(Filter::Bucket(id)) = self.folders.get(i).map(|a| a.filter.clone()) {
                    if let Some(at) = s.hidden.iter().position(|h| *h == id) {
                        s.hidden.remove(at);
                    } else {
                        s.hidden.push(id);
                    }
                    return Action::Changed;
                }
            }
            Hit::Row(id) => match id {
                RowId::Accent => self.picker = Some(Picker::Accent),
                RowId::Grid => self.picker = Some(Picker::Grid),
                RowId::Hidden => {
                    self.page = Page::Hidden;
                    self.scroller.reset();
                }
                RowId::Manage => return Action::ManageMedia,
                RowId::Updates => return Action::Update,
                RowId::Internet => {}
            },
        }
        Action::None
    }

    /// A Holo tick box, 18 dp, centred on (cx, cy).
    fn tick_box(batch: &mut Batch, cx: f32, cy: f32, on: bool, d: f32) {
        let (bx, by, s) = (cx - 9.0 * d, cy - 9.0 * d, 18.0 * d);
        batch.rect(Rect::new(bx, by, s, s), Rgba::hex(BOX_FILL), 0.0);
        batch.outline(Rect::new(bx, by, s, s), Rgba::hex(BOX_EDGE), 0.0, 2.0 * d);
        if on {
            let (ox, oy) = (bx - 5.0 * d, by - 7.0 * d);
            let p = |x: f32, y: f32| (ox + x * d, oy + y * d);
            let accent = Rgba::hex(holo::accent());
            let (a, b, c) = (p(9.0, 15.0), p(14.0, 20.0), p(27.0, 5.0));
            holo::stroke(batch, a, b, 3.5 * d, accent);
            holo::stroke(batch, b, c, 3.5 * d, accent);
        }
    }

    /// Draw the screen. Returns whether it is still moving, and the folder
    /// covers it wants but has no picture for yet.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gl: &Context, r: &mut Renderer, batch: &mut Batch, thumbs: &Thumbs, f: Frame, s: &GallerySettings, manage: Option<bool>, now: Instant) -> (bool, Vec<(i64, bool, u16)>) {
        let d = self.d;
        self.last = Some(f);
        self.lay_out(r, s, manage, f.w);
        let (top, bottom) = (self.list_top(&f), self.list_bottom(&f));
        self.scroller.set_bounds((self.content_h() - (bottom - top)).max(0.0));
        let scroll = self.scroller.offset(now);
        let white = Rgba::hex(WHITE);
        let accent = Rgba::hex(holo::accent());
        let w = f.w;
        let mut missing = Vec::new();
        batch.rect(Rect::new(0.0, 0.0, w, f.h), Rgba::hex(holo::BLACK), 0.0);

        match self.page {
            Page::Main => {
                for b in &self.blocks {
                    match b {
                        Block::Section { title, y } => {
                            let y = top + y - scroll;
                            if y > bottom || y + 41.0 * d < top {
                                continue;
                            }
                            let em = 14.0 * d;
                            r.text_spaced(gl, batch, title, 20.0 * d, y + 20.0 * d + r.ascent(em), em, 0.6 * d, w - 40.0 * d, accent, Weight::Bold);
                            batch.rect(Rect::new(16.0 * d, y + 40.0 * d, w - 32.0 * d, 1.0 * d), accent, 0.0);
                        }
                        Block::Row { id, y, h: rh, sub } => {
                            let y = top + y - scroll;
                            if y > bottom || y + rh < top {
                                continue;
                            }
                            let (title, _, tick) = row_text(*id, s, manage);
                            if self.pressed == Some(Hit::Row(*id)) {
                                batch.rect(Rect::new(0.0, y, w, *rh), accent.with_alpha(0.25), 0.0);
                            }
                            let block = (27.0 + 19.0 * sub.len() as f32) * d;
                            let pad = (rh - block) / 2.0;
                            let max = w - 40.0 * d - if tick.is_some() { 60.0 * d } else { 0.0 };
                            r.text(gl, batch, title, 20.0 * d, y + pad + 19.0 * d, 18.0 * d, max, white, Weight::Regular);
                            for (i, line) in sub.iter().enumerate() {
                                r.text(gl, batch, line, 20.0 * d, y + pad + 42.0 * d + i as f32 * 19.0 * d, 14.0 * d, max, Rgba::hex(SUB), Weight::Regular);
                            }
                            if let Some(on) = tick {
                                Self::tick_box(batch, w - 42.0 * d, y + rh / 2.0, on, d);
                            }
                            batch.rect(Rect::new(16.0 * d, y + rh - 1.0 * d, w - 32.0 * d, 1.0 * d), Rgba::hex(ROW_LINE), 0.0);
                        }
                    }
                }
            }
            Page::Hidden => {
                let rh = self.album_row_h();
                let pic = 48.0 * d;
                let hidden: HashSet<i64> = s.hidden.iter().copied().collect();
                for (i, a) in self.folders.iter().enumerate() {
                    let y = top + i as f32 * rh - scroll;
                    if y > bottom || y + rh < top {
                        continue;
                    }
                    if self.pressed == Some(Hit::Album(i)) {
                        batch.rect(Rect::new(0.0, y, w, rh), accent.with_alpha(0.25), 0.0);
                    }
                    let rc = Rect::new(16.0 * d, y + (rh - pic) / 2.0, pic, pic);
                    match r.icon_uv(&thumbs::key(a.cover)) {
                        Some(uv) => batch.image(rc, uv, 0.0, 1.0),
                        None => {
                            batch.rect(rc, Rgba::hex(TILE_EMPTY), 0.0);
                            if let Some(&(video, orientation)) = self.covers.get(&a.cover)
                                && !thumbs.failed(a.cover)
                            {
                                missing.push((a.cover, video, orientation));
                            }
                        }
                    }
                    let max = w - 160.0 * d;
                    r.text(gl, batch, &a.name, 80.0 * d, y + rh / 2.0 - 2.0 * d, 18.0 * d, max, white, Weight::Regular);
                    let count = if a.count == 1 { "1 item".to_string() } else { format!("{} items", a.count) };
                    r.text(gl, batch, &count, 80.0 * d, y + rh / 2.0 + 18.0 * d, 14.0 * d, max, Rgba::hex(SUB), Weight::Regular);
                    let on = matches!(a.filter, Filter::Bucket(id) if hidden.contains(&id));
                    Self::tick_box(batch, w - 42.0 * d, y + rh / 2.0, on, d);
                    batch.rect(Rect::new(16.0 * d, y + rh - 1.0 * d, w - 32.0 * d, 1.0 * d), Rgba::hex(ROW_LINE), 0.0);
                }
                if self.folders.is_empty() {
                    let em = 17.0 * d;
                    let msg = "No folders yet";
                    let mw = r.measure(msg, em, w, Weight::Light);
                    r.text(gl, batch, msg, (w - mw) / 2.0, top + 120.0 * d, em, w, Rgba::hex(SUB), Weight::Light);
                }
            }
        }

        // Cover what scrolled behind the bars.
        batch.rect(Rect::new(0.0, bottom, w, f.h - bottom), Rgba::hex(holo::BLACK), 0.0);
        let title = if self.page == Page::Hidden { "Hidden albums" } else { "Settings" };
        chrome::back_bar(gl, r, batch, w, f.ins.top, title, self.pressed == Some(Hit::Back), d);

        if self.picker.is_some() {
            batch.rect(Rect::new(0.0, 0.0, w, f.h), Rgba::hex(holo::BLACK).with_alpha(0.55), 0.0);
            let want = match self.picker {
                Some(Picker::Accent) => RowId::Accent,
                _ => RowId::Grid,
            };
            let row_y = self.blocks.iter().find_map(|b| match b {
                Block::Row { id, y, .. } if *id == want => Some(top + y - scroll),
                _ => None,
            });
            let choices = self.choices(s);
            let rows: Vec<MenuRow> = choices.iter().map(|(t, on)| MenuRow::Radio(t, *on)).collect();
            let pw = 260.0 * d;
            let ph = rows.iter().map(|row| row.height(d)).sum::<f32>() + 8.0 * d;
            let py = row_y.unwrap_or(top).clamp(top, (bottom - ph - 8.0 * d).max(top));
            let px = 16.0 * d;
            let pressed = match self.pressed {
                Some(Hit::Choice(i)) => Some(i),
                _ => None,
            };
            self.picker_rows = widgets::popup(gl, r, batch, px, py, pw, &rows, pressed, d);
            self.picker_x = (px, pw);
            if self.picker == Some(Picker::Accent) {
                for (i, a) in Accent::ALL.iter().enumerate() {
                    if let Some(&(ry, rh)) = self.picker_rows.get(i) {
                        batch.circle(px + pw - 62.0 * d, ry + rh / 2.0, 9.0 * d, Rgba::hex(a.rgb()));
                    }
                }
            }
        }
        (self.scroller.animating(now) || self.scroller.moved(), missing)
    }
}
