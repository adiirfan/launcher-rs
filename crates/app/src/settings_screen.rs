//! "Home settings": the launcher's own settings screen, and its Hidden apps
//! page. Slides in from the right over Home.

use std::time::{Duration, Instant};

use android_activity::AndroidApp;
use launcher_core::geometry::Insets;
use launcher_core::scroll::{ListGesture, Scroller};
use launcher_core::settings::{Accent, Settings};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::fonts::Weight;
use crate::holo::{self, BLACK, DIVIDER, ROW_LINE, WHITE};
use crate::home::{Home, Touch, TouchKind};
use crate::icons::Icons;
use crate::listener;
use crate::menu::{self, R4, inside};
use crate::render::Renderer;

const OPEN: Duration = Duration::from_millis(220);
const CLOSE: Duration = Duration::from_millis(180);
const HEADER_BG: u32 = 0x141414;
const SUB: u32 = 0x9E9E9E;
const BOX_FILL: u32 = 0x1A1A1A;
const BOX_EDGE: u32 = 0x8C8C8C;

#[derive(Clone, Copy, Debug, PartialEq)]
enum RowId {
    Theme,
    Accent,
    Icons,
    Labels,
    Clock,
    Music,
    AddNew,
    Search,
    Hidden,
    Dots,
    Animations,
    Glow,
    Internet,
}

const SECTIONS: [(&str, &[RowId]); 5] = [
    ("LOOK", &[RowId::Theme, RowId::Accent, RowId::Icons, RowId::Labels]),
    ("HOME SCREEN", &[RowId::Clock, RowId::Music, RowId::AddNew, RowId::Search]),
    ("APPS", &[RowId::Hidden, RowId::Dots]),
    ("SPEED", &[RowId::Animations, RowId::Glow]),
    ("PRIVACY", &[RowId::Internet]),
];

/// A row's title, its line underneath, and its tick box (if it has one).
fn row_text(id: RowId, s: &Settings, access: bool) -> (&'static str, String, Option<bool>) {
    const NEEDS_ACCESS: &str = "Needs notification access. Tap to allow.";
    match id {
        RowId::Theme => ("Theme", "Holo (the Android 4 look)".into(), None),
        RowId::Accent => ("Accent colour", s.accent.name().into(), None),
        RowId::Icons => ("App icons", "Each app shows its own icon".into(), None),
        RowId::Labels => ("Icon labels", "Show names under icons on Home".into(), Some(s.labels)),
        RowId::Clock => ("Clock widget", "Time, date and your next event".into(), Some(s.events)),
        RowId::Music => (
            "Music controls",
            if s.music && !access { NEEDS_ACCESS } else { "Play, pause and skip from Home" }.into(),
            Some(s.music),
        ),
        RowId::AddNew => (
            "Add new apps to Home",
            if s.add_new_apps { "New installs also go on Home" } else { "New installs only go in the app list" }.into(),
            Some(s.add_new_apps),
        ),
        RowId::Search => ("Search bar", "Apps, contacts and settings on this phone".into(), None),
        RowId::Hidden => ("Hidden apps", "Choose apps to keep out of the list".into(), None),
        RowId::Dots => (
            "Notification dots",
            if s.dots && !access { NEEDS_ACCESS } else { "Show a dot when an app has something new" }.into(),
            Some(s.dots),
        ),
        RowId::Animations => ("Animations", s.animations.name().into(), None),
        RowId::Glow => ("Scrolling glow", "A glow at the end of a list".into(), Some(s.glow)),
        RowId::Internet => ("Internet access", "Not requested. The launcher works fully offline".into(), None),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Page {
    Main,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Back,
    Row(RowId),
    App(usize),
    /// A colour in the accent menu.
    Colour(usize),
    Outside,
}

/// One block of the main page, in list coordinates (before scrolling).
enum Block {
    Section { title: &'static str, y: f32 },
    Row { id: RowId, y: f32, h: f32, sub: Vec<String> },
}

#[derive(Clone, Copy, Debug)]
struct Slide {
    from: f32,
    to: f32,
    start: Instant,
    dur: Duration,
}

pub struct SettingsScreen {
    shown: f32,
    slide: Option<Slide>,
    page: Page,
    scroller: Scroller,
    pressed: Option<Hit>,
    /// The accent colour menu is open.
    picker: bool,
    blocks: Vec<Block>,
    /// Settings and width the blocks were laid out for.
    laid_out: Option<(Settings, f32, bool)>,
    screen: (f32, f32),
    insets: Insets,
    density: f32,
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

impl SettingsScreen {
    pub fn new(density: f32) -> SettingsScreen {
        SettingsScreen {
            shown: 0.0,
            slide: None,
            page: Page::Main,
            scroller: Scroller::new(8.0 * density),
            pressed: None,
            picker: false,
            blocks: Vec::new(),
            laid_out: None,
            screen: (0.0, 0.0),
            insets: Insets { top: 0.0, bottom: 0.0, left: 0.0, right: 0.0 },
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

    pub fn is_open(&self) -> bool {
        self.slide.map_or(self.shown, |s| s.to) > 0.0
    }

    pub fn covers_home(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    pub fn open(&mut self, now: Instant, speed: f32) {
        let from = self.progress(now);
        self.slide = Some(Slide { from, to: 1.0, start: now, dur: OPEN.mul_f32(speed) });
        self.page = Page::Main;
        self.picker = false;
        self.pressed = None;
        self.scroller.reset();
        log::info!("settings opened");
    }

    pub fn close(&mut self, now: Instant, speed: f32) {
        let from = self.progress(now);
        self.slide = (speed > 0.0).then_some(Slide { from, to: 0.0, start: now, dur: CLOSE.mul_f32(speed) });
        self.shown = 0.0;
        self.picker = false;
        self.pressed = None;
    }

    /// Back key or the back arrow: close the colour menu, then the Hidden
    /// apps page, then the screen.
    pub fn back(&mut self, now: Instant, speed: f32) {
        if self.picker {
            self.picker = false;
        } else if self.page == Page::Hidden {
            self.page = Page::Main;
            self.scroller.reset();
        } else {
            self.close(now, speed);
        }
    }

    fn header(&self) -> R4 {
        (0.0, self.insets.top, self.screen.0, 56.0 * self.density)
    }

    fn list_top(&self) -> f32 {
        let h = self.header();
        h.1 + h.3
    }

    fn list_bottom(&self) -> f32 {
        self.screen.1 - self.insets.bottom
    }

    fn app_row_h(&self) -> f32 {
        64.0 * self.density
    }

    /// The accent row's place on screen, for the colour menu.
    fn picker_rect(&self, now: Instant) -> Option<R4> {
        let scroll = self.scroller.offset(now);
        let (y, h) = self.blocks.iter().find_map(|b| match b {
            Block::Row { id: RowId::Accent, y, h, .. } => Some((*y, *h)),
            _ => None,
        })?;
        let d = self.density;
        let anchor = (16.0 * d, self.list_top() + y - scroll, 240.0 * d, h);
        Some(menu::popup_rect(anchor, Accent::ALL.len(), 240.0, self.screen, self.insets, d))
    }

    fn hit(&self, x: f32, y: f32, now: Instant, apps: usize) -> Hit {
        let d = self.density;
        if self.picker {
            return match self.picker_rect(now).and_then(|p| menu::popup_hit(p, Accent::ALL.len(), x, y, d)) {
                Some(i) => Hit::Colour(i),
                None => Hit::Outside,
            };
        }
        let h = self.header();
        if inside(h, x, y) {
            return if x < 72.0 * d { Hit::Back } else { Hit::Outside };
        }
        if y < self.list_top() || y >= self.list_bottom() {
            return Hit::Outside;
        }
        let ly = y - self.list_top() + self.scroller.offset(now);
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
                let i = (ly / self.app_row_h()) as usize;
                if ly >= 0.0 && i < apps { Hit::App(i) } else { Hit::Outside }
            }
        }
    }

    /// True when the screen needs drawing.
    pub fn touch(&mut self, t: Touch, app: &AndroidApp, home: &mut Home) -> bool {
        if !self.is_open() || self.screen.0 == 0.0 {
            return false;
        }
        let apps = home.all_apps().len();
        match t.kind {
            TouchKind::Down => {
                self.pressed = Some(self.hit(t.x, t.y, t.t, apps));
                if !self.picker {
                    self.scroller.touch_down(t.x, t.y, t.t);
                }
            }
            TouchKind::Move => {
                if !self.picker {
                    self.scroller.touch_move(t.x, t.y, t.t);
                    if self.scroller.moved() {
                        self.pressed = None;
                    }
                }
            }
            TouchKind::Cancel => {
                self.scroller.touch_cancel(t.t);
                self.pressed = None;
            }
            TouchKind::Up => {
                let pressed = self.pressed.take();
                let tap = if self.picker {
                    true
                } else {
                    matches!(self.scroller.touch_up(t.x, t.y, t.t), ListGesture::Tap { .. })
                };
                let now_hit = self.hit(t.x, t.y, t.t, apps);
                if tap && pressed == Some(now_hit) {
                    self.act(now_hit, app, home, t.t);
                }
            }
        }
        true
    }

    fn act(&mut self, hit: Hit, app: &AndroidApp, home: &mut Home, now: Instant) {
        let mut s = home.settings();
        match hit {
            Hit::Back => self.back(now, s.animations.factor()),
            Hit::Colour(i) => {
                s.accent = Accent::ALL[i];
                self.picker = false;
                log::info!("settings: accent {}", s.accent.name());
            }
            Hit::Outside => self.picker = false,
            Hit::App(i) => {
                if let Some(key) = home.all_apps().get(i).map(|r| r.key.clone()) {
                    home.toggle_hidden(&key);
                }
            }
            Hit::Row(id) => match id {
                RowId::Accent => self.picker = true,
                RowId::Labels => s.labels = !s.labels,
                RowId::AddNew => s.add_new_apps = !s.add_new_apps,
                RowId::Dots | RowId::Music => {
                    let on = if id == RowId::Dots { &mut s.dots } else { &mut s.music };
                    let access = home.listener_access();
                    // On but not allowed yet: the row says "Tap to allow", so a
                    // tap shows Android's page for it instead of switching off.
                    if !(*on && !access) {
                        *on = !*on;
                    }
                    if *on && !access {
                        listener::open_access_settings(app);
                    }
                }
                RowId::Clock => home.toggle_events(app),
                RowId::Glow => s.glow = !s.glow,
                RowId::Animations => s.animations = s.animations.next(),
                RowId::Hidden => {
                    self.page = Page::Hidden;
                    self.scroller.reset();
                }
                // These rows only say what the launcher does.
                RowId::Theme | RowId::Icons | RowId::Search | RowId::Internet => {}
            },
        }
        // toggle_events may have changed the settings itself.
        if hit != Hit::Row(RowId::Clock) && s != home.settings() {
            log::info!("settings changed");
            home.set_settings(s);
        }
    }

    /// Text broken into lines no wider than `max`.
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

    fn lay_out(&mut self, r: &mut Renderer, s: &Settings, access: bool) {
        if self.laid_out == Some((*s, self.screen.0, access)) {
            return;
        }
        let d = self.density;
        let mut blocks = Vec::new();
        let mut y = 0.0;
        for (title, rows) in SECTIONS {
            blocks.push(Block::Section { title, y });
            y += 41.0 * d;
            for &id in rows {
                let (_, sub, tick) = row_text(id, s, access);
                let max = self.screen.0 - 40.0 * d - if tick.is_some() { 60.0 * d } else { 0.0 };
                let sub = Self::wrap(r, &sub, 14.0 * d, max);
                let h = (51.0 + 19.0 * sub.len() as f32).max(72.0) * d;
                blocks.push(Block::Row { id, y, h, sub });
                y += h;
            }
        }
        self.blocks = blocks;
        self.laid_out = Some((*s, self.screen.0, access));
    }

    fn content_height(&self, apps: usize) -> f32 {
        match self.page {
            Page::Main => self.blocks.last().map_or(0.0, |b| match b {
                Block::Row { y, h, .. } => y + h,
                Block::Section { y, .. } => *y,
            }) + 40.0 * self.density,
            Page::Hidden => apps as f32 * self.app_row_h() + 24.0 * self.density,
        }
    }

    /// A Holo tick box, 18 dp, centred on (cx, cy).
    fn tick_box(batch: &mut Batch, cx: f32, cy: f32, on: bool, d: f32, fade: f32) {
        let (bx, by, s) = (cx - 9.0 * d, cy - 9.0 * d, 18.0 * d);
        batch.rect(Rect::new(bx, by, s, s), Rgba::hex(BOX_FILL).with_alpha(fade), 0.0);
        batch.outline(Rect::new(bx, by, s, s), Rgba::hex(BOX_EDGE).with_alpha(fade), 0.0, 2.0 * d);
        if on {
            // The tick overshoots the box at the top right, as Holo's did.
            let (ox, oy) = (bx - 5.0 * d, by - 7.0 * d);
            let p = |x: f32, y: f32| (ox + x * d, oy + y * d);
            let accent = Rgba::hex(holo::accent()).with_alpha(fade);
            let (a, b, c) = (p(9.0, 15.0), p(14.0, 20.0), p(27.0, 5.0));
            holo::stroke(batch, a, b, 3.5 * d, accent);
            holo::stroke(batch, b, c, 3.5 * d, accent);
        }
    }

    /// Build the screen's part of the frame. True while anything moves.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, icons: &mut Icons, home: &Home, screen: (i32, i32), insets: Insets, now: Instant) -> bool {
        let p = self.progress(now);
        if let Some(s) = self.slide
            && now >= s.start + s.dur
        {
            self.shown = s.to;
            self.slide = None;
        }
        if p <= 0.0 {
            return false;
        }
        self.screen = (screen.0 as f32, screen.1 as f32);
        self.insets = insets;
        let d = self.density;
        let settings = home.settings();
        let access = home.listener_access();
        self.lay_out(r, &settings, access);
        let apps = home.all_apps();
        let visible = self.list_bottom() - self.list_top();
        self.scroller.set_bounds(self.content_height(apps.len()) - visible);
        let (w, h) = self.screen;
        let dx = (1.0 - p) * w;
        let fade = 1.0;
        let white = Rgba::hex(WHITE);
        let accent = Rgba::hex(holo::accent());
        let scroll = self.scroller.offset(now);
        let top = self.list_top();

        batch.rect(Rect::new(dx, 0.0, w, h), Rgba::hex(BLACK), 0.0);
        match self.page {
            Page::Main => {
                for b in &self.blocks {
                    match b {
                        Block::Section { title, y } => {
                            let y = top + y - scroll;
                            if y > h || y + 41.0 * d < top {
                                continue;
                            }
                            let em = 14.0 * d;
                            r.text_spaced(gl, batch, title, dx + 20.0 * d, y + 20.0 * d + r.ascent(em), em, 0.6 * d, w - 40.0 * d, accent, Weight::Bold);
                            batch.rect(Rect::new(dx + 16.0 * d, y + 40.0 * d, w - 32.0 * d, 1.0 * d), accent, 0.0);
                        }
                        Block::Row { id, y, h: rh, sub } => {
                            let y = top + y - scroll;
                            if y > h || y + rh < top {
                                continue;
                            }
                            let (title, _, tick) = row_text(*id, &settings, access);
                            if self.pressed == Some(Hit::Row(*id)) {
                                batch.rect(Rect::new(dx, y, w, *rh), accent.with_alpha(0.25), 0.0);
                            }
                            let block = (27.0 + 19.0 * sub.len() as f32) * d;
                            let pad = (rh - block) / 2.0;
                            let max = w - 40.0 * d - if tick.is_some() { 60.0 * d } else { 0.0 };
                            r.text(gl, batch, title, dx + 20.0 * d, y + pad + 19.0 * d, 18.0 * d, max, white, Weight::Regular);
                            for (i, line) in sub.iter().enumerate() {
                                r.text(gl, batch, line, dx + 20.0 * d, y + pad + 42.0 * d + i as f32 * 19.0 * d, 14.0 * d, max, Rgba::hex(SUB), Weight::Regular);
                            }
                            if let Some(on) = tick {
                                Self::tick_box(batch, dx + w - 42.0 * d, y + rh / 2.0, on, d, fade);
                            }
                            batch.rect(Rect::new(dx + 16.0 * d, y + rh - 1.0 * d, w - 32.0 * d, 1.0 * d), Rgba::hex(ROW_LINE), 0.0);
                        }
                    }
                }
            }
            Page::Hidden => {
                let rh = self.app_row_h();
                let first = (scroll / rh).floor().max(0.0) as usize;
                let last = (((scroll + visible) / rh).ceil() as usize).min(apps.len());
                let icon = 40.0 * d;
                for (i, rec) in apps.iter().enumerate().take(last).skip(first) {
                    let y = top + i as f32 * rh - scroll;
                    if self.pressed == Some(Hit::App(i)) {
                        batch.rect(Rect::new(dx, y, w, rh), accent.with_alpha(0.25), 0.0);
                    }
                    icons.draw(gl, r, batch, &rec.key, (dx + 20.0 * d, y + (rh - icon) / 2.0, icon), false, 1.0);
                    let em = 18.0 * d;
                    r.text(gl, batch, &rec.label, dx + 76.0 * d, y + rh / 2.0 + r.ascent(em) / 2.0 - 1.0 * d, em, w - 156.0 * d, white, Weight::Regular);
                    Self::tick_box(batch, dx + w - 42.0 * d, y + rh / 2.0, home.is_hidden(&rec.key), d, fade);
                    batch.rect(Rect::new(dx + 16.0 * d, y + rh - 1.0 * d, w - 32.0 * d, 1.0 * d), Rgba::hex(ROW_LINE), 0.0);
                }
            }
        }

        // The Holo glow at whichever end is being pushed past.
        let over = self.scroller.overscroll();
        if settings.glow && over != 0.0 {
            let strength = (over.abs() / (160.0 * d)).min(1.0);
            let y = if over < 0.0 { top } else { self.list_bottom() };
            batch.glow(Rect::new(dx + w * 0.15, y, w * 0.7, 1.0), accent.with_alpha(0.55 * strength), 0.0, 28.0 * d * (0.5 + strength));
        }

        // Cover what scrolled behind the header and the navigation bar.
        batch.rect(Rect::new(dx, 0.0, w, top), Rgba::hex(BLACK), 0.0);
        batch.rect(Rect::new(dx, self.list_bottom(), w, h - self.list_bottom()), Rgba::hex(BLACK), 0.0);

        // Header: back arrow, the accent square, the title.
        let (_, hy, _, hh) = self.header();
        batch.rect(Rect::new(dx, hy, w, hh), Rgba::hex(HEADER_BG), 0.0);
        batch.rect(Rect::new(dx, hy + hh - 1.0 * d, w, 1.0 * d), Rgba::hex(DIVIDER), 0.0);
        let cy = hy + hh / 2.0;
        if self.pressed == Some(Hit::Back) {
            batch.rect(Rect::new(dx, hy, 72.0 * d, hh - 1.0 * d), accent.with_alpha(0.25), 0.0);
        }
        holo::stroke(batch, (dx + 26.0 * d, cy - 7.0 * d), (dx + 19.0 * d, cy), 2.0 * d, white);
        holo::stroke(batch, (dx + 19.0 * d, cy), (dx + 26.0 * d, cy + 7.0 * d), 2.0 * d, white);
        let (sx, sy, ss) = (dx + 32.0 * d, cy - 16.0 * d, 32.0 * d);
        batch.rect(Rect::new(sx, sy, ss, ss), accent, 4.0 * d);
        for i in 0..4 {
            let (col, row) = ((i % 2) as f32, (i / 2) as f32);
            batch.rect(Rect::new(sx + 9.0 * d + col * 8.0 * d, sy + 9.0 * d + row * 8.0 * d, 6.0 * d, 6.0 * d), white, 0.0);
        }
        let title = match self.page {
            Page::Main => "Home settings",
            Page::Hidden => "Hidden apps",
        };
        let em = 20.0 * d;
        r.text(gl, batch, title, dx + 76.0 * d, cy + r.ascent(em) / 2.0 - 1.0 * d, em, w - 92.0 * d, white, Weight::Regular);

        // The accent colour menu, each colour with a swatch.
        if self.picker
            && let Some(pr) = self.picker_rect(now)
        {
            batch.rect(Rect::new(dx, 0.0, w, h), Rgba::hex(BLACK).with_alpha(0.55), 0.0);
            let names: Vec<String> = Accent::ALL.iter().map(|a| a.name().to_string()).collect();
            let pressed = match self.pressed {
                Some(Hit::Colour(i)) => Some(i),
                _ => None,
            };
            menu::draw_popup(gl, r, batch, pr, &names, pressed, false, d);
            for (i, a) in Accent::ALL.iter().enumerate() {
                let (cx, cy) = (pr.0 + pr.2 - 30.0 * d, pr.1 + 1.0 * d + (i as f32 + 0.5) * 52.0 * d);
                batch.circle(cx, cy, 11.0 * d, Rgba::hex(a.rgb()));
                if *a == settings.accent {
                    batch.outline(Rect::new(cx - 15.0 * d, cy - 15.0 * d, 30.0 * d, 30.0 * d), white, 15.0 * d, 2.0 * d);
                }
            }
        }

        self.slide.is_some_and(|s| now < s.start + s.dur) || self.scroller.animating(now) || self.scroller.moved()
    }
}
