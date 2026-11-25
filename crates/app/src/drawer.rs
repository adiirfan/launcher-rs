//! The app list: every app A to Z, and search across apps, settings pages
//! and contacts. It slides up over Home.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use android_activity::AndroidApp;
use launcher_core::applist::AppRecord;
use launcher_core::drawer_geometry::{COLS, DrawerMetrics};
use launcher_core::geometry::Insets;
use launcher_core::home::AppKey;
use launcher_core::scroll::{ListGesture, Scroller};
use launcher_core::search::{self, SettingsPage};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::contacts::{self, Contact};
use crate::home::{Touch, TouchKind};
use crate::icons::Icons;
use crate::render::Renderer;
use crate::settings_pages;

const OPEN: Duration = Duration::from_millis(220);
const CLOSE: Duration = Duration::from_millis(180);
const PULL_CLOSE_DP: f32 = 80.0;
const MAX_CONTACTS: usize = 5;

const SURFACE2: u32 = 0x101917;
const ON: u32 = 0xE3EEEA;
const MUTED: u32 = 0xA9BDB6;
const ACCENT: u32 = 0x9CC9B8;
const DOT_BG: u32 = 0x14211E;

#[derive(Clone, Debug, PartialEq)]
pub enum DrawerAction {
    None,
    Launch(AppKey),
    Close,
    ShowKeyboard,
    ClearText,
    OpenSettings(&'static str),
    OpenContact(Contact),
    AskContacts,
    OpenAppDetails,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Permission {
    Unknown,
    Granted,
    Missing,
    /// Refused; `can_ask` says whether Android will still show its question.
    Refused { can_ask: bool },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Settings(usize),
    Contact(usize),
    ContactsPrompt,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Field,
    Clear,
    Back,
    App(usize),
    Row(Row),
}

#[derive(Clone, Copy, Debug)]
struct Slide {
    from: f32,
    to: f32,
    start: Instant,
    dur: Duration,
}

pub struct Drawer {
    /// 0 = hidden, 1 = fully open; `slide` animates between them.
    shown: f32,
    slide: Option<Slide>,
    query: String,
    focused: bool,
    /// Indexes into the app records matching `query`.
    results: Vec<usize>,
    results_gen: Option<u64>,
    settings: Vec<&'static SettingsPage>,
    available: Option<HashSet<&'static str>>,
    contacts: Vec<Contact>,
    contacts_for: Option<String>,
    permission: Permission,
    /// Check whether the permission was turned on elsewhere (in Settings).
    recheck: bool,
    scroller: Scroller,
    metrics: Option<(DrawerMetrics, (i32, i32), Insets, u32)>,
    pressed: Option<Target>,
    density: f32,
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

impl Drawer {
    pub fn new(density: f32) -> Drawer {
        Drawer {
            shown: 0.0,
            slide: None,
            query: String::new(),
            focused: false,
            results: Vec::new(),
            results_gen: None,
            settings: Vec::new(),
            available: None,
            contacts: Vec::new(),
            contacts_for: None,
            permission: Permission::Unknown,
            recheck: false,
            scroller: Scroller::new(8.0 * density),
            metrics: None,
            pressed: None,
            density,
        }
    }

    fn progress(&self, now: Instant) -> f32 {
        match self.slide {
            Some(s) if now < s.start + s.dur => {
                let t = (now - s.start).as_secs_f32() / s.dur.as_secs_f32();
                s.from + (s.to - s.from) * ease_out(t)
            }
            Some(s) => s.to,
            None => self.shown,
        }
    }

    /// True when the list fully covers Home and nothing is sliding.
    pub fn covers_home(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    pub fn is_open(&self) -> bool {
        self.slide.map_or(self.shown, |s| s.to) > 0.0
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.slide.is_some_and(|s| now < s.start + s.dur) || self.scroller.animating(now) || self.scroller.moved()
    }

    pub fn open(&mut self, now: Instant, focus: bool) {
        let from = self.progress(now);
        self.slide = Some(Slide { from, to: 1.0, start: now, dur: OPEN });
        self.focused = focus;
        self.scroller.reset();
        self.permission = Permission::Unknown;
    }

    /// Slide away (or vanish at once, after launching an app) and forget the search.
    pub fn close(&mut self, now: Instant, animate: bool) {
        let from = self.progress(now);
        self.slide = animate.then_some(Slide { from, to: 0.0, start: now, dur: CLOSE });
        self.shown = 0.0;
        self.focused = false;
        self.pressed = None;
        self.set_query("");
    }

    pub fn set_query(&mut self, q: &str) {
        if q != self.query {
            self.query = q.to_string();
            self.results_gen = None;
            self.scroller.reset();
        }
    }

    pub fn has_query(&self) -> bool {
        !self.query.trim().is_empty()
    }

    fn update_results(&mut self, apps: &[AppRecord], generation: u64) {
        if self.results_gen == Some(generation) {
            return;
        }
        let labels: Vec<&str> = apps.iter().map(|a| a.label.as_str()).collect();
        self.results = search::rank(&labels, &self.query);
        self.settings = match &self.available {
            Some(av) => search::match_settings(&self.query, &|a| av.contains(a)),
            None => Vec::new(),
        };
        self.results_gen = Some(generation);
    }

    /// Slow lookups, done after a frame is on screen. True if a redraw is needed.
    pub fn after_frame(&mut self, app: &AndroidApp) -> bool {
        if !self.is_open() {
            return false;
        }
        let mut redraw = false;
        if self.permission == Permission::Unknown {
            self.permission = if contacts::has_permission(app) { Permission::Granted } else { Permission::Missing };
            redraw = true;
        } else if self.recheck && self.permission != Permission::Granted && contacts::has_permission(app) {
            self.permission = Permission::Granted;
            self.contacts_for = None;
            redraw = true;
        }
        self.recheck = false;
        if !self.has_query() {
            return redraw;
        }
        if self.available.is_none() {
            self.available = Some(settings_pages::available(app));
            self.results_gen = None;
            redraw = true;
        }
        if self.permission == Permission::Granted && self.contacts_for.as_deref() != Some(self.query.as_str()) {
            self.contacts = contacts::search(app, &self.query, MAX_CONTACTS);
            self.contacts_for = Some(self.query.clone());
            redraw = true;
        }
        redraw
    }

    /// Android answered the contacts question.
    pub fn permission_answer(&mut self, app: &AndroidApp, granted: bool) {
        self.permission = if granted {
            Permission::Granted
        } else {
            Permission::Refused { can_ask: contacts::can_ask_again(app) }
        };
        self.contacts_for = None;
    }

    /// Back on screen (from Settings, say): see if the permission was turned
    /// on. A refusal that just arrived is kept, not reset.
    pub fn recheck_permission(&mut self) {
        self.recheck = true;
    }

    /// Enter on the keyboard: the best app match, else the best settings page.
    pub fn first_match(&self, apps: &[AppRecord]) -> DrawerAction {
        if !self.has_query() {
            return DrawerAction::None;
        }
        if let Some(&i) = self.results.first() {
            return DrawerAction::Launch(apps[i].key.clone());
        }
        self.settings.first().map_or(DrawerAction::None, |p| DrawerAction::OpenSettings(p.action))
    }

    fn rows(&self) -> Vec<Row> {
        if !self.has_query() {
            return Vec::new();
        }
        let mut rows: Vec<Row> = (0..self.settings.len()).map(Row::Settings).collect();
        if self.permission == Permission::Granted {
            rows.extend((0..self.contacts.len()).map(Row::Contact));
        } else {
            rows.push(Row::ContactsPrompt);
        }
        rows
    }

    fn row_h(&self) -> f32 {
        56.0 * self.density
    }

    /// Where the extras (no-match line, rows, footnote) start, below the grid,
    /// in list coordinates (before scrolling).
    fn extras_layout(&self, m: &DrawerMetrics) -> (f32, f32, f32, f32) {
        let d = self.density;
        let grid_rows = self.results.len().div_ceil(COLS);
        let grid_end = m.pad_top + grid_rows as f32 * m.row_h;
        let no_match = if self.has_query() && self.results.is_empty() { 40.0 * d } else { 0.0 };
        let rows_top = grid_end + no_match + 24.0 * d;
        let rows_h = self.rows().len() as f32 * self.row_h();
        (grid_end, rows_top, rows_h, rows_top + rows_h + 14.0 * d + 20.0 * d)
    }

    fn metrics_for(&mut self, screen: (i32, i32), insets: Insets, icon: u32) -> DrawerMetrics {
        match self.metrics {
            Some((m, s, i, ic)) if s == screen && i == insets && ic == icon => m,
            _ => {
                let m = DrawerMetrics::new(screen.0 as f32, screen.1 as f32, self.density, insets, icon as f32);
                self.metrics = Some((m, screen, insets, icon));
                m
            }
        }
    }

    fn update_bounds(&mut self, m: &DrawerMetrics) {
        let rows = self.results.len().div_ceil(COLS);
        let extra = if self.has_query() { self.extras_layout(m).3 - (m.pad_top + rows as f32 * m.row_h) } else { 0.0 };
        let content = m.content_height(rows, extra);
        self.scroller.set_bounds(content - m.visible_height());
    }

    fn inside(r: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
        x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
    }

    fn clear_rect(&self, m: &DrawerMetrics) -> (f32, f32, f32, f32) {
        let s = 48.0 * self.density;
        (m.field.0 + m.field.2 - s - 4.0 * self.density, m.field.1 + (m.field.3 - s) / 2.0, s, s)
    }

    fn target_at(&self, x: f32, y: f32, now: Instant) -> Option<Target> {
        let (m, ..) = self.metrics?;
        if self.has_query() && Self::inside(self.clear_rect(&m), x, y) {
            return Some(Target::Clear);
        }
        if Self::inside(m.field, x, y) {
            return Some(Target::Field);
        }
        if Self::inside(m.back_button, x, y) {
            return Some(Target::Back);
        }
        let scroll = self.scroller.offset(now);
        if let Some(i) = m.hit_cell(x, y, scroll, self.results.len()) {
            return Some(Target::App(i));
        }
        if y < m.list_top {
            return None;
        }
        let (_, rows_top, rows_h, _) = self.extras_layout(&m);
        let ly = y - m.list_top + scroll - rows_top;
        let side = 8.0 * self.density + m.list_left;
        if ly >= 0.0 && ly < rows_h && x >= side && x < m.screen_w - side {
            return self.rows().get((ly / self.row_h()) as usize).map(|&r| Target::Row(r));
        }
        None
    }

    pub fn touch(&mut self, t: Touch, apps: &[AppRecord]) -> DrawerAction {
        if self.metrics.is_none() || !self.is_open() {
            return DrawerAction::None;
        }
        match t.kind {
            TouchKind::Down => {
                self.pressed = self.target_at(t.x, t.y, t.t);
                self.scroller.touch_down(t.x, t.y, t.t);
                DrawerAction::None
            }
            TouchKind::Move => {
                self.scroller.touch_move(t.x, t.y, t.t);
                if self.scroller.moved() {
                    self.pressed = None;
                }
                DrawerAction::None
            }
            TouchKind::Cancel => {
                self.scroller.touch_cancel(t.t);
                self.pressed = None;
                DrawerAction::None
            }
            TouchKind::Up => {
                let pressed = self.pressed.take();
                match self.scroller.touch_up(t.x, t.y, t.t) {
                    ListGesture::PullDown { dy } if dy > PULL_CLOSE_DP * self.density => DrawerAction::Close,
                    ListGesture::Tap { x, y } if pressed.is_some() && pressed == self.target_at(x, y, t.t) => {
                        self.tap(pressed.unwrap(), apps)
                    }
                    _ => DrawerAction::None,
                }
            }
        }
    }

    fn tap(&mut self, target: Target, apps: &[AppRecord]) -> DrawerAction {
        match target {
            Target::Field => {
                self.focused = true;
                DrawerAction::ShowKeyboard
            }
            Target::Clear => {
                self.set_query("");
                DrawerAction::ClearText
            }
            Target::Back => DrawerAction::Close,
            Target::App(i) => self.results.get(i).map_or(DrawerAction::None, |&a| DrawerAction::Launch(apps[a].key.clone())),
            Target::Row(Row::Settings(i)) => self.settings.get(i).map_or(DrawerAction::None, |p| DrawerAction::OpenSettings(p.action)),
            Target::Row(Row::Contact(i)) => self.contacts.get(i).map_or(DrawerAction::None, |c| DrawerAction::OpenContact(c.clone())),
            Target::Row(Row::ContactsPrompt) => match self.permission {
                Permission::Refused { can_ask: false } => DrawerAction::OpenAppDetails,
                _ => DrawerAction::AskContacts,
            },
        }
    }

    /// Build the list's part of the frame. Returns true while anything moves.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, icons: &mut Icons, apps: (&[AppRecord], u64), screen: (i32, i32), insets: Insets, now: Instant) -> bool {
        let p = self.progress(now);
        if let Some(s) = self.slide
            && now >= s.start + s.dur
        {
            self.shown = s.to;
            self.slide = None;
        }
        if p <= 0.0 {
            return self.animating(now);
        }
        let (records, generation) = apps;
        self.update_results(records, generation);
        let m = self.metrics_for(screen, insets, icons.size());
        self.update_bounds(&m);
        let d = self.density;
        let fade = p;
        let lift = (1.0 - p) * 0.15 * m.screen_h;
        let scroll = self.scroller.offset(now);
        let (sw, sh) = (m.screen_w, m.screen_h);

        batch.rect(Rect::new(0.0, 0.0, sw, sh), Rgba::hex(0x000000).with_alpha(fade), 0.0);

        // The grid: only rows that are at least partly on screen (the list
        // shows through the navigation bar, so down to the screen's edge).
        let label_em = 12.5 * d;
        let label_ascent = r.ascent(label_em);
        let label_max = m.cell_w - 8.0 * d;
        let top = scroll - m.pad_top;
        let first_row = (top / m.row_h).floor().max(0.0) as usize;
        let last_row = ((top + sh - m.list_top - lift) / m.row_h).floor().max(0.0) as usize;
        let start = first_row * COLS;
        let end = ((last_row + 1) * COLS).min(self.results.len());
        for slot in start..end {
            let rec = &records[self.results[slot]];
            let (ix, iy, is, _) = m.icon_rect(slot, scroll);
            let iy = iy + lift;
            let pressed = self.pressed == Some(Target::App(slot));
            icons.draw(gl, r, batch, &rec.key, (ix, iy, is), pressed, fade);
            if !rec.label.is_empty() {
                let lw = r.measure(&rec.label, label_em, label_max, false);
                let baseline = iy + is + m.label_gap + label_ascent;
                r.text(gl, batch, &rec.label, ix + is / 2.0 - lw / 2.0, baseline, label_em, label_max, Rgba::hex(ON).with_alpha(fade), false);
            }
        }

        // Below the grid when searching: no-match line, settings, contacts, footnote.
        if self.has_query() {
            let (grid_end, rows_top, rows_h, _) = self.extras_layout(&m);
            let y0 = m.list_top - scroll + lift;
            let left = m.list_left + 8.0 * d;
            let width = sw - 2.0 * left;
            let quoted = format!("\u{201C}{}\u{201D}", self.query.trim());
            if self.results.is_empty() {
                let text = format!("No apps called {quoted}");
                r.text(gl, batch, &text, left, y0 + grid_end + 8.0 * d + r.ascent(15.0 * d), 15.0 * d, width, Rgba::hex(MUTED).with_alpha(fade), false);
            }
            let rows = self.rows();
            let top = y0 + rows_top;
            batch.rect(Rect::new(left, top, width, rows_h), Rgba::hex(SURFACE2).with_alpha(fade), 20.0 * d);
            for (i, row) in rows.iter().enumerate() {
                let ry = top + i as f32 * self.row_h();
                if i > 0 {
                    batch.rect(Rect::new(left, ry - 1.0 * d, width, 2.0 * d), Rgba::hex(0x000000).with_alpha(fade), 0.0);
                }
                let pressed = self.pressed == Some(Target::Row(*row));
                let ink = Rgba::hex(ON).with_alpha(fade * if pressed { 0.6 } else { 1.0 });
                let (cx, cy) = (left + 16.0 * d + 14.0 * d, ry + self.row_h() / 2.0);
                let text_x = cx + 14.0 * d + 14.0 * d;
                let baseline = cy + r.ascent(15.0 * d) / 2.0 - 1.0 * d;
                let max = left + width - text_x - 16.0 * d;
                let accent = Rgba::hex(ACCENT).with_alpha(fade);
                match *row {
                    Row::Settings(s) => {
                        let rr = 9.0 * d;
                        batch.outline(Rect::new(cx - rr, cy - rr, rr * 2.0, rr * 2.0), accent, rr, 2.5 * d);
                        batch.circle(cx, cy, 2.5 * d, accent);
                        r.text(gl, batch, self.settings[s].label, text_x, baseline, 15.0 * d, max, ink, false);
                    }
                    Row::Contact(c) => {
                        let name = &self.contacts[c].name;
                        batch.circle(cx, cy, 14.0 * d, Rgba::hex(DOT_BG).with_alpha(fade));
                        let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                        let iw = r.measure(&initial, 14.0 * d, 28.0 * d, true);
                        r.text(gl, batch, &initial, cx - iw / 2.0, cy + r.ascent(14.0 * d) / 2.0 - 1.0 * d, 14.0 * d, 28.0 * d, accent, true);
                        r.text(gl, batch, name, text_x, baseline, 15.0 * d, max, ink, false);
                    }
                    Row::ContactsPrompt => {
                        let rr = 6.0 * d;
                        batch.outline(Rect::new(cx - rr, cy - rr - 3.0 * d, rr * 2.0, rr * 2.0), accent, rr, 2.0 * d);
                        batch.rect(Rect::new(cx - 9.0 * d, cy + 5.0 * d, 18.0 * d, 7.0 * d), accent, 4.0 * d);
                        let text = match self.permission {
                            Permission::Refused { can_ask: true } => "Contacts are off. Tap to allow.".to_string(),
                            Permission::Refused { can_ask: false } => "Contacts are off. Tap for Settings.".to_string(),
                            _ => format!("Search contacts for {quoted}"),
                        };
                        r.text(gl, batch, &text, text_x, baseline, 15.0 * d, max, ink, false);
                    }
                }
            }
            let note_y = top + rows_h + 14.0 * d + r.ascent(13.0 * d);
            r.text(gl, batch, "Search stays on this phone. Nothing is sent to the web.", left, note_y, 13.0 * d, width, Rgba::hex(MUTED).with_alpha(fade), false);
        }

        // Cover whatever scrolled up behind the search field.
        batch.rect(Rect::new(0.0, 0.0, sw, m.list_top + lift), Rgba::hex(0x000000).with_alpha(fade), 0.0);

        // Search field.
        let (fx, fy, fw, fh) = m.field;
        let fy = fy + lift;
        batch.rect(Rect::new(fx, fy, fw, fh), Rgba::hex(SURFACE2).with_alpha(fade), fh / 2.0);
        let muted = Rgba::hex(MUTED).with_alpha(fade);
        let (gx, gy) = (fx + 20.0 * d + 7.0 * d, fy + fh / 2.0 - 1.5 * d);
        let lens = 7.0 * d;
        batch.outline(Rect::new(gx - lens, gy - lens, lens * 2.0, lens * 2.0), muted, lens, 2.0 * d);
        for step in [1.0f32, 1.6, 2.2] {
            let o = lens * 0.72 + step * 2.0 * d;
            batch.circle(gx + o, gy + o, 1.4 * d, muted);
        }
        let em = 16.0 * d;
        let tx = gx + lens + 12.0 * d;
        let baseline = fy + fh / 2.0 + r.ascent(em) / 2.0 - 1.0 * d;
        let text_max = fx + fw - tx - 56.0 * d;
        let typed_w = if self.query.is_empty() {
            r.text(gl, batch, "Search apps", tx, baseline, em, text_max, Rgba::hex(MUTED).with_alpha(fade * 0.75), false);
            0.0
        } else {
            r.text(gl, batch, &self.query, tx, baseline, em, text_max, Rgba::hex(ON).with_alpha(fade), false)
        };
        if self.focused {
            let ch = 20.0 * d;
            batch.rect(Rect::new(tx + typed_w + 1.0 * d, fy + (fh - ch) / 2.0, 2.0 * d, ch), Rgba::hex(ACCENT).with_alpha(fade), 0.0);
        }
        if self.has_query() {
            let (cx, cy, cw, ch) = self.clear_rect(&m);
            let em = 20.0 * d;
            let xw = r.measure("\u{00D7}", em, cw, false);
            let alpha = if self.pressed == Some(Target::Clear) { 0.5 } else { 1.0 };
            r.text(gl, batch, "\u{00D7}", cx + (cw - xw) / 2.0, cy + lift + ch / 2.0 + r.ascent(em) / 2.0 - 1.0 * d, em, cw, Rgba::hex(MUTED).with_alpha(fade * alpha), false);
        }

        // Back to Home.
        let (bx, by, bw, bh) = m.back_button;
        let by = by + lift;
        let alpha = if self.pressed == Some(Target::Back) { 0.7 } else { 1.0 };
        batch.rect(Rect::new(bx, by, bw, bh), Rgba::hex(SURFACE2).with_alpha(fade * alpha), bh / 2.0);
        let em = 14.0 * d;
        let label = "Back to Home";
        let lw = r.measure(label, em, bw, true);
        r.text(gl, batch, label, bx + (bw - lw) / 2.0, by + bh / 2.0 + r.ascent(em) / 2.0 - 1.0 * d, em, bw, Rgba::hex(ON).with_alpha(fade), true);

        self.animating(now)
    }
}
