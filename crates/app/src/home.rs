//! The modern home screen: date, pages of icons, dock, search pill.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use android_activity::AndroidApp;
use launcher_core::applist::AppRecord;
use launcher_core::geometry::{DOCK_SLOTS, HomeMetrics, Hit, Insets};
use launcher_core::home::{AppKey, HomeLayout};
use launcher_core::pager::{Gesture, Pager};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::apps;
use crate::cache::Store;
use crate::render::Renderer;
use crate::system;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TouchKind {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Clone, Copy, Debug)]
pub struct Touch {
    pub kind: TouchKind,
    pub x: f32,
    pub y: f32,
    pub t: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Launch(AppKey),
    OpenNotifications,
    OpenAppList,
    OpenSearch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Cell { page: usize, col: u32, row: u32 },
    Dock(u32),
    Pill,
}

/// A vertical swipe this long (dp) opens the shade or the app list.
const SWIPE_DP: f32 = 80.0;
const LABEL_DP: f32 = 12.5;
const DATE_DP: f32 = 28.0;

pub struct Home {
    store: Store,
    layout: HomeLayout,
    labels: HashMap<AppKey, String>,
    seq: i32,
    boot: Option<i32>,
    pager: Pager,
    metrics: Option<(HomeMetrics, (i32, i32), Insets)>,
    pressed: Option<Target>,
    date: String,
    /// Icons Android couldn't draw; shown as a plain circle, not retried.
    failed_icons: HashSet<AppKey>,
    /// Icons not on disk yet: fetched from Android after the frame.
    missing_icons: Vec<AppKey>,
    /// Icons whose app changed: dropped from the atlas next frame.
    stale_icons: Vec<AppKey>,
    density: f32,
}

fn atlas_key(key: &AppKey) -> String {
    format!("{}/{}", key.package, key.activity)
}

impl Home {
    /// Everything from the files on disk. Only a first install (no saved
    /// app list) asks Android here, before the first frame.
    pub fn load(app: &AndroidApp) -> Home {
        let t0 = Instant::now();
        let store = Store::open(app);
        let density = app.config().density().unwrap_or(160) as f32 / 160.0;
        let (records, seq, boot) = match store.load_apps() {
            Some((records, seq)) => (records, seq, store.load_boot()),
            None => {
                let seq = apps::current_seq(app);
                let records = apps::list_apps(app);
                let boot = apps::boot_count(app);
                store.save_apps(&records, seq);
                store.save_boot(boot);
                log::info!("first run: listed {} apps live", records.len());
                (records, seq, Some(boot))
            }
        };
        let layout = store.load_layout().unwrap_or_else(|| {
            let layout = apps::first_run_layout(app, &records);
            store.save_layout(&layout);
            log::info!("first run: dock {:?}", layout.dock.iter().map(|k| k.as_ref().map(|k| k.package.as_str())).collect::<Vec<_>>());
            layout
        });
        let mut layout = layout;
        let installed: HashSet<AppKey> = records.iter().map(|r| r.key.clone()).collect();
        if layout.remove_missing(&installed) {
            store.save_layout(&layout);
            log::info!("layout: removed missing apps");
        }
        let labels = records.into_iter().map(|r| (r.key, r.label)).collect();
        let pager = Pager::new(layout.pages.len(), 1.0);
        let date = system::date_text(app).unwrap_or_default();
        log::info!("home loaded in {:.1} ms", t0.elapsed().as_secs_f64() * 1e3);
        Home {
            store,
            layout,
            labels,
            seq,
            boot,
            pager,
            metrics: None,
            pressed: None,
            date,
            failed_icons: HashSet::new(),
            missing_icons: Vec::new(),
            stale_icons: Vec::new(),
            density,
        }
    }

    /// Ask Android what changed since the saved list. Returns true when
    /// something on screen may have changed.
    pub fn refresh(&mut self, app: &AndroidApp) -> bool {
        let t0 = Instant::now();
        let boot = apps::boot_count(app);
        let (seq, changed) = if self.boot == Some(boot) {
            match apps::changed_since(app, self.seq) {
                None => return false,
                Some((seq, pkgs)) => (seq, pkgs),
            }
        } else {
            // New boot: Android's change counter started over, so list again.
            (apps::current_seq(app), Vec::new())
        };
        let records = apps::list_apps(app);
        if records.is_empty() {
            log::warn!("refresh: app list came back empty, keeping the old one");
            return false;
        }
        let changed: HashSet<&str> = changed.iter().map(String::as_str).collect();
        for key in self.labels.keys().chain(records.iter().map(|r| &r.key)) {
            if changed.contains(key.package.as_str()) {
                self.store.forget_icon(key);
                self.failed_icons.remove(key);
                self.stale_icons.push(key.clone());
            }
        }
        let installed: HashSet<AppKey> = records.iter().map(|r| r.key.clone()).collect();
        if self.layout.remove_missing(&installed) {
            self.store.save_layout(&self.layout);
            log::info!("layout: removed missing apps");
            self.pager.set_pages(self.layout.pages.len());
        }
        self.store.save_apps(&records, seq);
        self.store.save_boot(boot);
        self.labels = records.into_iter().map(|AppRecord { key, label }| (key, label)).collect();
        self.seq = seq;
        self.boot = Some(boot);
        log::info!("refresh: {} changed package(s), {:.0} ms", changed.len(), t0.elapsed().as_secs_f64() * 1e3);
        true
    }

    /// Re-read today's date. True if it changed.
    pub fn update_date(&mut self, app: &AndroidApp) -> bool {
        match system::date_text(app) {
            Some(d) if d != self.date => {
                self.date = d;
                true
            }
            _ => false,
        }
    }

    /// Fetch icons that weren't on disk, save them, and say whether to redraw.
    pub fn load_missing_icons(&mut self, app: &AndroidApp) -> bool {
        let Some((m, ..)) = self.metrics else { return false };
        let size = m.icon.round() as i32;
        let missing = std::mem::take(&mut self.missing_icons);
        for key in &missing {
            match apps::load_icon_rgba(app, key, size) {
                Some((px, w, _)) => self.store.save_icon(key, w as u32, &px),
                None => {
                    self.failed_icons.insert(key.clone());
                }
            }
        }
        !missing.is_empty()
    }

    fn metrics_for(&mut self, screen: (i32, i32), insets: Insets) -> HomeMetrics {
        match self.metrics {
            Some((m, s, i)) if s == screen && i == insets => m,
            _ => {
                let m = HomeMetrics::new(screen.0 as f32, screen.1 as f32, self.density, insets, self.layout.cols, self.layout.rows);
                log::info!("home metrics: icon {:.0}px cell {:.0}x{:.0} insets {:?}", m.icon, m.cell_w, m.cell_h, insets);
                self.metrics = Some((m, screen, insets));
                self.pager.set_width(screen.0 as f32);
                // Icons at a new size: the saved ones no longer match.
                self.missing_icons.clear();
                m
            }
        }
    }

    fn target_at(&self, x: f32, y: f32) -> Option<Target> {
        let (m, ..) = self.metrics?;
        Some(match m.hit(x, y)? {
            Hit::Cell { col, row } => Target::Cell { page: self.pager.page(), col, row },
            Hit::Dock(i) => Target::Dock(i),
            Hit::Pill => Target::Pill,
        })
    }

    fn app_at(&self, t: Target) -> Option<&AppKey> {
        match t {
            Target::Cell { page, col, row } => self.layout.slot(page, col, row),
            Target::Dock(i) => self.layout.dock.get(i as usize)?.as_ref(),
            Target::Pill => None,
        }
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        if self.metrics.is_none() {
            return Action::None;
        }
        match t.kind {
            TouchKind::Down => {
                self.pager.touch_down(t.x, t.y, t.t);
                self.pressed = self.target_at(t.x, t.y).filter(|&tg| tg == Target::Pill || self.app_at(tg).is_some());
                Action::None
            }
            TouchKind::Move => {
                self.pager.touch_move(t.x, t.y, t.t);
                if self.pager.moved() {
                    self.pressed = None;
                }
                Action::None
            }
            TouchKind::Up => {
                let pressed = self.pressed.take();
                let swipe = SWIPE_DP * self.density;
                match self.pager.touch_up(t.x, t.y, t.t) {
                    Gesture::Tap { x, y } => match (pressed, self.target_at(x, y)) {
                        (Some(Target::Pill), Some(Target::Pill)) => Action::OpenSearch,
                        (Some(p), Some(now)) if p == now => self.app_at(p).cloned().map_or(Action::None, Action::Launch),
                        _ => Action::None,
                    },
                    Gesture::VerticalSwipe { dy } if dy > swipe => Action::OpenNotifications,
                    Gesture::VerticalSwipe { dy } if dy < -swipe => Action::OpenAppList,
                    _ => Action::None,
                }
            }
            TouchKind::Cancel => {
                self.pager.touch_cancel(t.t);
                self.pressed = None;
                Action::None
            }
        }
    }

    /// Queue one icon: from the atlas, else from disk, else ask for it later.
    fn icon_uv(&mut self, gl: &glow::Context, r: &mut Renderer, key: &AppKey, size: u32) -> Option<[f32; 4]> {
        let ak = atlas_key(key);
        if let Some(uv) = r.icon_uv(&ak) {
            return Some(uv);
        }
        if self.failed_icons.contains(key) {
            return None;
        }
        match self.store.load_icon(key, size) {
            Some((px, w, h)) => r.icon(gl, &ak, &px, w, h),
            None => {
                if !self.missing_icons.contains(key) {
                    self.missing_icons.push(key.clone());
                }
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_icon(&mut self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, key: &AppKey, ir: (f32, f32, f32, f32), size: u32, pressed: bool) {
        let (mut x, mut y, mut s) = (ir.0, ir.1, ir.2);
        let mut opacity = 1.0;
        if pressed {
            let shrunk = s * 0.92;
            x += (s - shrunk) / 2.0;
            y += (s - shrunk) / 2.0;
            s = shrunk;
            opacity = 0.7;
        }
        match self.icon_uv(gl, r, key, size) {
            Some(uv) => batch.image(Rect::new(x, y, s, s), uv, s / 2.0, opacity),
            None => batch.circle(x + s / 2.0, y + s / 2.0, s / 2.0, Rgba::hex(0x737373).with_alpha(opacity)),
        }
    }

    /// Text with a soft dark shadow so it reads on light wallpapers.
    #[allow(clippy::too_many_arguments)]
    fn shadowed(&self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, s: &str, x: f32, baseline: f32, em: f32, max_w: f32, bold: bool) {
        let d = self.density;
        r.text(gl, batch, s, x + 0.5 * d, baseline + 1.0 * d, em, max_w, Rgba::hex(0x000000).with_alpha(0.45), bold);
        r.text(gl, batch, s, x, baseline, em, max_w, Rgba::hex(0xFFFFFF), bold);
    }

    /// Build the frame. Returns true while something is still moving.
    pub fn draw(&mut self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, screen: (i32, i32), insets: Insets, now: Instant) -> bool {
        let m = self.metrics_for(screen, insets);
        let d = self.density;
        let size = m.icon.round() as u32;
        let m_icon = size as f32;

        for key in std::mem::take(&mut self.stale_icons) {
            r.forget_icon(&atlas_key(&key));
        }

        // Date.
        let date = self.date.clone();
        self.shadowed(gl, r, batch, &date, m.date_left, m.date_baseline, DATE_DP * d, m.screen_w - m.date_left * 2.0, false);

        // Pages: the current one, plus its neighbour while swiping.
        let offset = self.pager.offset(now);
        let label_em = LABEL_DP * d;
        let label_ascent = r.ascent(label_em);
        let label_max = m.cell_w - 8.0 * d;
        for page in 0..self.layout.pages.len() {
            let shift = page as f32 * m.screen_w - offset;
            if shift.abs() >= m.screen_w {
                continue;
            }
            for row in 0..m.rows {
                for col in 0..m.cols {
                    let Some(key) = self.layout.slot(page, col, row).cloned() else { continue };
                    let ir = m.icon_rect(col, row);
                    let ir = (ir.0 + shift, ir.1, m_icon, m_icon);
                    let pressed = self.pressed == Some(Target::Cell { page, col, row });
                    self.draw_icon(gl, r, batch, &key, ir, size, pressed);
                    let label = self.labels.get(&key).cloned().unwrap_or_default();
                    if !label.is_empty() {
                        let lw = r.measure(&label, label_em, label_max, false);
                        let cx = ir.0 + m_icon / 2.0;
                        let baseline = ir.1 + m_icon + m.label_gap + label_ascent;
                        self.shadowed(gl, r, batch, &label, cx - lw / 2.0, baseline, label_em, label_max, false);
                    }
                }
            }
        }

        // Page dots, only when there's more than one page.
        let pages = self.layout.pages.len();
        if pages > 1 {
            let gap = 12.0 * d;
            let radius = 3.0 * d;
            let current = (offset / m.screen_w).round().clamp(0.0, (pages - 1) as f32) as usize;
            let x0 = m.screen_w / 2.0 - gap * (pages - 1) as f32 / 2.0;
            for i in 0..pages {
                let alpha = if i == current { 1.0 } else { 0.4 };
                batch.circle(x0 + gap * i as f32, m.dots_y, radius, Rgba::hex(0xFFFFFF).with_alpha(alpha));
            }
        }

        // Dock, no labels.
        for i in 0..DOCK_SLOTS {
            let Some(key) = self.layout.dock[i as usize].clone() else { continue };
            let pressed = self.pressed == Some(Target::Dock(i));
            self.draw_icon(gl, r, batch, &key, m.dock_icon_rect(i), size, pressed);
        }

        // Search pill.
        let (px, py, pw, ph) = m.pill;
        let pill_alpha = if self.pressed == Some(Target::Pill) { 0.7 } else { 0.85 };
        batch.rect(Rect::new(px, py, pw, ph), Rgba::hex(0x101413).with_alpha(pill_alpha), ph / 2.0);
        let ink = Rgba::hex(0xE3EEEA).with_alpha(0.9);
        let (gx, gy) = (px + 22.0 * d, py + ph / 2.0 - 1.5 * d);
        let lens = 7.0 * d;
        batch.outline(Rect::new(gx - lens, gy - lens, lens * 2.0, lens * 2.0), ink, lens, 2.0 * d);
        for step in [1.0f32, 1.6, 2.2] {
            let o = lens * 0.72 + step * 2.0 * d;
            batch.circle(gx + o, gy + o, 1.4 * d, ink);
        }
        let hint_em = 15.0 * d;
        let baseline = py + ph / 2.0 + r.ascent(hint_em) / 2.0 - 1.0 * d;
        let hint_x = gx + lens + 14.0 * d;
        r.text(gl, batch, "Search apps, contacts, settings", hint_x, baseline, hint_em, px + pw - hint_x - 20.0 * d, ink, false);

        self.pager.animating(now) || self.pager.dragging_pages()
    }

    pub fn has_missing_icons(&self) -> bool {
        !self.missing_icons.is_empty()
    }
}
