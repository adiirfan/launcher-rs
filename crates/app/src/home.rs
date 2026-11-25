//! The Holo home screen: clock, pages of icons, dock with the all-apps
//! button, and the search bar along the bottom.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use android_activity::AndroidApp;
use launcher_core::applist::AppRecord;
use launcher_core::geometry::{ALL_APPS_SLOT, CLOCK_SP, DOCK_SLOTS, HOLO_COLS, HOLO_ROWS, HomeMetrics, Hit, Insets};
use launcher_core::home::{AppKey, HomeLayout, Item, Pos};
use launcher_core::longpress::{LONG_PRESS, LongPress};
use launcher_core::pager::{Gesture, Pager};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::apps::{self, Shortcut};
use crate::cache::Store;
use crate::folder::{self, Panel};
use crate::fonts::Weight;
use crate::holo::{self, ACCENT, BLACK, WHITE};
use crate::icons::Icons;
use crate::menu::{self, R4, inside};
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
    StartShortcut(Shortcut),
    AppInfo(AppKey),
    Uninstall(AppKey),
    PickWallpaper,
    HomeSettings,
    /// Start typing a folder's new name (show the keyboard).
    StartRename,
    /// The name is saved; put the keyboard away.
    EndRename,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Cell { page: usize, col: u32, row: u32 },
    Dock(u32),
    AllApps,
    Search,
}

/// Where a lifted item came from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    Home(Pos),
    Folder { page: usize, col: u32, row: u32, index: usize },
    List,
}

/// An item picked up by a long-press.
struct Lift {
    source: Source,
    item: Item,
    /// The layout before anything moved: put back if the drop fails.
    before: HomeLayout,
    /// Where its icon was (x, y, size), for the menu.
    origin: (f32, f32, f32),
    shortcuts: Vec<Shortcut>,
    /// Already taken out of the layout (once dragging starts).
    taken: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MenuHit {
    Bar(usize),
    Row(usize),
    Icon,
    Outside,
}

/// The finger while a menu is open.
#[derive(Clone, Copy, Debug)]
enum Finger {
    /// Still down from the long-press that opened the menu.
    Held { x0: f32, y0: f32 },
    /// A new touch on the open menu.
    Tap { x0: f32, y0: f32, pressed: MenuHit },
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum FolderHit {
    App(usize),
    Name,
    Panel,
    Outside,
}

struct OpenFolder {
    page: usize,
    col: u32,
    row: u32,
    /// The new name being typed.
    renaming: Option<String>,
    pressed: Option<FolderHit>,
}

enum Mode {
    Normal,
    /// Long-press took hold: dimmed Home, the action bar, the shortcuts.
    Menu { lift: Lift, finger: Finger },
    Drag { lift: Lift, x: f32, y: f32, edge: Option<(i8, Instant)> },
    /// Long-press on empty space: Wallpapers and Home settings.
    Space { x: f32, y: f32, finger: Finger },
    Folder(OpenFolder),
}

/// What a long-press would pick up.
#[derive(Clone, Copy, Debug)]
enum PressAt {
    Item(Pos),
    Space,
    FolderApp(usize),
}

/// A vertical swipe this long (dp) opens the shade or the app list.
const SWIPE_DP: f32 = 80.0;
/// How far (dp) a finger may wander before a press stops being a long-press.
const PRESS_SLOP_DP: f32 = 12.0;
/// Hold a dragged icon this close (dp) to a side to change page...
const EDGE_DP: f32 = 24.0;
/// ...for this long.
const EDGE_HOLD: Duration = Duration::from_millis(500);
const LABEL_SP: f32 = 13.0;
const DATE_SP: f32 = 14.0;

pub struct Home {
    store: Store,
    layout: HomeLayout,
    /// Every launchable app, A to Z, hidden ones included.
    all_records: Vec<AppRecord>,
    /// The same, less hidden apps: what the app list and search show.
    records: Vec<AppRecord>,
    /// Bumped whenever `records` changes.
    records_gen: u64,
    labels: HashMap<AppKey, String>,
    seq: i32,
    boot: Option<i32>,
    pager: Pager,
    metrics: Option<(HomeMetrics, (i32, i32), Insets)>,
    pressed: Option<Target>,
    /// Hours, ":minutes", and the date in capitals.
    clock: (String, String, String),
    density: f32,
    mode: Mode,
    press: LongPress,
    press_at: Option<(PressAt, f32, f32)>,
}

impl Home {
    /// Everything from the files on disk. Only a first install (no saved
    /// app list) asks Android here, before the first frame.
    pub fn load(app: &AndroidApp, store: Store) -> Home {
        let t0 = Instant::now();
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
        if layout.reflow(HOLO_COLS, HOLO_ROWS) {
            store.save_layout(&layout);
            log::info!("layout: reflowed to {HOLO_COLS}x{HOLO_ROWS}, {} page(s)", layout.pages.len());
        }
        let installed: HashSet<AppKey> = records.iter().map(|r| r.key.clone()).collect();
        if layout.remove_missing(&installed) {
            store.save_layout(&layout);
            log::info!("layout: removed missing apps");
        }
        let labels = records.iter().map(|r| (r.key.clone(), r.label.clone())).collect();
        let pager = Pager::new(layout.pages.len(), 1.0);
        let clock = system::clock_text(app).unwrap_or_default();
        log::info!("home loaded in {:.1} ms", t0.elapsed().as_secs_f64() * 1e3);
        let visible = visible_records(&records, &layout);
        Home {
            store,
            layout,
            all_records: records,
            records: visible,
            records_gen: 0,
            labels,
            seq,
            boot,
            pager,
            metrics: None,
            pressed: None,
            clock,
            density,
            mode: Mode::Normal,
            press: LongPress::new(LONG_PRESS, PRESS_SLOP_DP * density),
            press_at: None,
        }
    }

    /// Ask Android what changed since the saved list. Returns true when
    /// something on screen may have changed.
    pub fn refresh(&mut self, app: &AndroidApp, icons: &mut Icons) -> bool {
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
                icons.changed(key);
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
        self.labels = records.iter().map(|AppRecord { key, label }| (key.clone(), label.clone())).collect();
        self.all_records = records;
        self.rebuild_visible();
        self.seq = seq;
        self.boot = Some(boot);
        log::info!("refresh: {} changed package(s), {:.0} ms", changed.len(), t0.elapsed().as_secs_f64() * 1e3);
        true
    }

    /// Re-read the time and date. True if what's shown changed.
    pub fn update_clock(&mut self, app: &AndroidApp) -> bool {
        match system::clock_text(app) {
            Some(c) if c != self.clock => {
                self.clock = c;
                true
            }
            _ => false,
        }
    }

    /// Every launchable app, A to Z, and a number that changes when it does.
    pub fn apps(&self) -> (&[AppRecord], u64) {
        (&self.records, self.records_gen)
    }

    fn rebuild_visible(&mut self) {
        self.records = visible_records(&self.all_records, &self.layout);
        self.records_gen += 1;
    }

    /// Home pressed while already Home: close whatever is open (a drag
    /// goes back where it came from) and slide back to the first page.
    pub fn go_to_first_page(&mut self, now: Instant) {
        self.pressed = None;
        self.close(now);
        self.pager.snap_to(0, now);
    }

    /// Back: close the menu or folder. A name being typed is saved.
    pub fn back(&mut self, now: Instant) -> Action {
        let renaming = self.renaming();
        self.close(now);
        if renaming { Action::EndRename } else { Action::None }
    }

    fn close(&mut self, _now: Instant) {
        self.press.cancel();
        self.press_at = None;
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Menu { lift, .. } | Mode::Drag { lift, .. } => self.restore(lift),
            Mode::Folder(f) => self.save_name(&f),
            _ => {}
        }
    }

    pub fn renaming(&self) -> bool {
        matches!(&self.mode, Mode::Folder(f) if f.renaming.is_some())
    }

    pub fn set_rename_text(&mut self, text: &str) {
        if let Mode::Folder(f) = &mut self.mode
            && let Some(name) = f.renaming.as_mut()
        {
            name.clear();
            name.push_str(text);
        }
    }

    /// Enter on the keyboard: keep the typed name. True if one was being typed.
    pub fn finish_rename(&mut self) -> bool {
        let Mode::Folder(f) = &mut self.mode else { return false };
        let Some(name) = f.renaming.take() else { return false };
        let (page, col, row) = (f.page, f.col, f.row);
        if !name.trim().is_empty() {
            self.layout.rename_folder(page, col, row, &name);
            self.store.save_layout(&self.layout);
            log::info!("folder renamed ({} chars)", name.chars().count());
        }
        true
    }

    fn save_name(&mut self, f: &OpenFolder) {
        if let Some(name) = f.renaming.as_deref().filter(|n| !n.trim().is_empty()) {
            self.layout.rename_folder(f.page, f.col, f.row, name);
            self.store.save_layout(&self.layout);
            log::info!("folder renamed ({} chars)", name.chars().count());
        }
    }

    /// Put the layout back as it was before the lift.
    fn restore(&mut self, lift: Lift) {
        self.layout = lift.before;
        self.pager.set_pages(self.layout.pages.len());
    }

    /// Keep the layout as it is now: tidy empty end pages and save once.
    fn commit(&mut self, why: &str) {
        self.layout.trim_empty_pages();
        self.pager.set_pages(self.layout.pages.len());
        self.store.save_layout(&self.layout);
        log::info!("layout: {why}, {} page(s)", self.layout.pages.len());
    }

    /// Take the lifted item out of the layout (once).
    fn take_out(&mut self, lift: &mut Lift) {
        if lift.taken {
            return;
        }
        lift.taken = true;
        match lift.source {
            Source::Home(pos) => {
                self.layout.take(pos);
            }
            Source::Folder { page, col, row, index } => {
                self.layout.take_from_folder(page, col, row, index);
            }
            Source::List => {}
        }
    }

    fn lift(&self, app: &AndroidApp, source: Source, item: Item, origin: (f32, f32, f32)) -> Lift {
        system::buzz(app);
        let shortcuts = match &item {
            Item::App(key) => apps::shortcuts(app, key),
            Item::Folder(_) => Vec::new(),
        };
        log::info!("lifted {} ({} shortcut(s))", item_name(&item), shortcuts.len());
        Lift { source, item, before: self.layout.clone(), origin, shortcuts, taken: false }
    }

    /// A long-press in the app list: the list has closed and the same finger
    /// carries the app on Home.
    pub fn lift_from_list(&mut self, app: &AndroidApp, key: AppKey, icon: R4, (x, y): (f32, f32)) {
        self.close(Instant::now());
        let lift = self.lift(app, Source::List, Item::App(key), (icon.0, icon.1, icon.2));
        self.mode = Mode::Menu { lift, finger: Finger::Held { x0: x, y0: y } };
    }

    /// Timers: a long-press taking hold, a dragged icon held at a side.
    /// True if the screen needs drawing.
    pub fn tick(&mut self, app: &AndroidApp, now: Instant) -> bool {
        let mut redraw = false;
        if self.press.fire(now)
            && let Some((at, x, y)) = self.press_at.take()
        {
            redraw = self.long_press(app, at, x, y, now);
        }
        if let Mode::Drag { edge: Some((side, since)), .. } = self.mode
            && now >= since + EDGE_HOLD
        {
            let page = self.pager.page();
            let to = if side < 0 { page.checked_sub(1) } else { Some(page + 1) };
            if let Some(to) = to {
                // A new page appears past the last one, unless the last is
                // already empty.
                let last_empty = self.layout.pages.get(page).is_some_and(|p| p.iter().all(Option::is_none));
                if to < self.layout.pages.len() || (!last_empty && to <= 64) {
                    self.layout.ensure_page(to);
                    self.pager.set_pages(self.layout.pages.len());
                    self.pager.snap_to(to, now);
                    redraw = true;
                }
            }
            if let Mode::Drag { edge, .. } = &mut self.mode {
                *edge = Some((side, now));
            }
        }
        redraw
    }

    pub fn next_wake(&self, now: Instant) -> Option<Duration> {
        let edge = match self.mode {
            Mode::Drag { edge: Some((_, since)), .. } => Some((since + EDGE_HOLD).saturating_duration_since(now)),
            _ => None,
        };
        [self.press.next_wake(now), edge].into_iter().flatten().min()
    }

    fn long_press(&mut self, app: &AndroidApp, at: PressAt, x: f32, y: f32, now: Instant) -> bool {
        let Some((m, ..)) = self.metrics else { return false };
        match (at, &self.mode) {
            (PressAt::Item(pos), Mode::Normal) => {
                let Some(item) = self.layout.item_at(pos) else { return false };
                self.pager.touch_cancel(now);
                self.pressed = None;
                let r = match pos {
                    Pos::Cell { col, row, .. } => m.icon_rect(col, row),
                    Pos::Dock(i) => m.dock_icon_rect(i),
                };
                let lift = self.lift(app, Source::Home(pos), item, (r.0, r.1, r.2));
                self.mode = Mode::Menu { lift, finger: Finger::Held { x0: x, y0: y } };
            }
            (PressAt::Space, Mode::Normal) => {
                self.pager.touch_cancel(now);
                self.pressed = None;
                system::buzz(app);
                self.mode = Mode::Space { x, y, finger: Finger::Held { x0: x, y0: y } };
            }
            (PressAt::FolderApp(index), Mode::Folder(f)) if f.renaming.is_none() => {
                let (page, col, row) = (f.page, f.col, f.row);
                let Some(Item::Folder(folder)) = self.layout.slot(page, col, row) else { return false };
                let Some(key) = folder.apps.get(index).cloned() else { return false };
                let r = self.panel(folder.apps.len()).icon_rect(index);
                let lift = self.lift(app, Source::Folder { page, col, row, index }, Item::App(key), (r.0, r.1, r.2));
                self.mode = Mode::Menu { lift, finger: Finger::Held { x0: x, y0: y } };
            }
            _ => return false,
        }
        true
    }

    fn panel(&self, apps: usize) -> Panel {
        let (m, _, insets) = self.metrics.expect("metrics before touches");
        let d = self.density;
        let top = (m.grid_top - 40.0 * d).max(insets.top + 8.0 * d);
        Panel::new(m.screen_w, top, m.icon, apps, d)
    }

    fn top_bar(&self) -> R4 {
        let (m, _, insets) = self.metrics.expect("metrics before touches");
        menu::top_bar(m.screen_w, insets, self.density)
    }

    /// The action bar's labels, and which ones do something, for this item.
    fn bar_labels(lift: &Lift) -> ([&'static str; 3], [bool; 3]) {
        let app = matches!(lift.item, Item::App(_));
        match lift.source {
            Source::List => (["UNINSTALL", "HIDE", "APP INFO"], [true, true, true]),
            _ => (["REMOVE", "HIDE", "APP INFO"], [true, app, app]),
        }
    }

    fn popup(&self, lift: &Lift) -> Option<R4> {
        if lift.shortcuts.is_empty() {
            return None;
        }
        let (m, _, insets) = self.metrics?;
        let (x, y, s) = lift.origin;
        Some(menu::popup_rect((x, y, s, s), lift.shortcuts.len(), 240.0, (m.screen_w, m.screen_h), insets, self.density))
    }

    fn space_popup(&self, x: f32, y: f32) -> R4 {
        let (m, _, insets) = self.metrics.expect("metrics before touches");
        menu::popup_rect((x, y, 0.0, 0.0), 2, 200.0, (m.screen_w, m.screen_h), insets, self.density)
    }

    fn menu_hit(&self, lift: &Lift, x: f32, y: f32) -> MenuHit {
        let d = self.density;
        if let Some(i) = menu::top_bar_hit(self.top_bar(), x, y) {
            return MenuHit::Bar(i);
        }
        if let Some(p) = self.popup(lift)
            && let Some(i) = menu::popup_hit(p, lift.shortcuts.len(), x, y, d)
        {
            return MenuHit::Row(i);
        }
        let (ix, iy, s) = lift.origin;
        if inside((ix, iy, s, s), x, y) {
            return MenuHit::Icon;
        }
        MenuHit::Outside
    }

    /// REMOVE (or UNINSTALL), HIDE, APP INFO.
    fn bar_action(&mut self, mut lift: Lift, i: usize) -> Action {
        let (_, enabled) = Self::bar_labels(&lift);
        if !enabled[i] {
            self.restore(lift);
            return Action::None;
        }
        let key = match &lift.item {
            Item::App(k) => Some(k.clone()),
            Item::Folder(_) => None,
        };
        match (i, lift.source, key) {
            (0, Source::List, Some(key)) => {
                self.restore(lift);
                Action::Uninstall(key)
            }
            (0, _, _) => {
                self.take_out(&mut lift);
                self.commit("removed from Home");
                Action::None
            }
            (1, _, Some(key)) => {
                self.layout = lift.before;
                self.layout.hide(&key);
                self.rebuild_visible();
                self.commit("app hidden");
                Action::None
            }
            (2, _, Some(key)) => {
                self.restore(lift);
                Action::AppInfo(key)
            }
            _ => {
                self.restore(lift);
                Action::None
            }
        }
    }

    /// The layout position under a dragged icon, on the page showing.
    fn drop_pos(&self, x: f32, y: f32) -> Option<Pos> {
        let (m, ..) = self.metrics?;
        match m.hit(x, y)? {
            Hit::Cell { col, row } => Some(Pos::Cell { page: self.pager.page(), col, row }),
            Hit::Dock(i) => Some(Pos::Dock(i)),
            _ => None,
        }
    }

    fn drop(&mut self, mut lift: Lift, x: f32, y: f32) -> Action {
        if let Some(i) = menu::top_bar_hit(self.top_bar(), x, y) {
            return self.bar_action(lift, i);
        }
        self.take_out(&mut lift);
        let Some(pos) = self.drop_pos(x, y) else {
            log::info!("drop: nowhere, put back");
            self.restore(lift);
            return Action::None;
        };
        match self.layout.drop_at(pos, lift.item.clone()) {
            Ok(()) => {
                self.commit("dropped");
                Action::None
            }
            Err(_) => {
                log::info!("drop: can't go there, put back");
                self.restore(lift);
                Action::None
            }
        }
    }

    fn metrics_for(&mut self, screen: (i32, i32), insets: Insets) -> HomeMetrics {
        match self.metrics {
            Some((m, s, i)) if s == screen && i == insets => m,
            _ => {
                let m = HomeMetrics::new(screen.0 as f32, screen.1 as f32, self.density, insets, self.layout.cols, self.layout.rows);
                log::info!("home metrics: icon {:.0}px cell {:.0}x{:.0} insets {:?}", m.icon, m.cell_w, m.cell_h, insets);
                self.metrics = Some((m, screen, insets));
                self.pager.set_width(screen.0 as f32);
                m
            }
        }
    }

    fn target_at(&self, x: f32, y: f32) -> Option<Target> {
        let (m, ..) = self.metrics?;
        Some(match m.hit(x, y)? {
            Hit::Cell { col, row } => Target::Cell { page: self.pager.page(), col, row },
            Hit::Dock(i) => Target::Dock(i),
            Hit::AllApps => Target::AllApps,
            Hit::Search => Target::Search,
        })
    }

    fn item_at(&self, t: Target) -> Option<Item> {
        match t {
            Target::Cell { page, col, row } => self.layout.item_at(Pos::Cell { page, col, row }),
            Target::Dock(i) if i != ALL_APPS_SLOT => self.layout.item_at(Pos::Dock(i)),
            _ => None,
        }
    }

    pub fn touch(&mut self, t: Touch) -> Action {
        if self.metrics.is_none() {
            return Action::None;
        }
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.touch_normal(t),
            Mode::Menu { lift, finger } => self.touch_menu(lift, finger, t),
            Mode::Drag { lift, x, y, edge } => self.touch_drag(lift, x, y, edge, t),
            Mode::Space { x, y, finger } => self.touch_space(x, y, finger, t),
            Mode::Folder(f) => self.touch_folder(f, t),
        }
    }

    fn touch_normal(&mut self, t: Touch) -> Action {
        match t.kind {
            TouchKind::Down => {
                self.pager.touch_down(t.x, t.y, t.t);
                let target = self.target_at(t.x, t.y);
                self.pressed = target.filter(|&tg| matches!(tg, Target::Search | Target::AllApps) || self.item_at(tg).is_some());
                let at = match target {
                    Some(Target::Search | Target::AllApps) => None,
                    Some(tg @ Target::Cell { page, col, row }) if self.item_at(tg).is_some() => Some(PressAt::Item(Pos::Cell { page, col, row })),
                    Some(tg @ Target::Dock(i)) if self.item_at(tg).is_some() => Some(PressAt::Item(Pos::Dock(i))),
                    _ => Some(PressAt::Space),
                };
                self.press_at = at.map(|a| (a, t.x, t.y));
                match at {
                    Some(_) => self.press.down(t.x, t.y, t.t),
                    None => self.press.cancel(),
                }
                Action::None
            }
            TouchKind::Move => {
                self.pager.touch_move(t.x, t.y, t.t);
                self.press.moved(t.x, t.y);
                if self.pager.moved() {
                    self.pressed = None;
                    self.press.cancel();
                }
                Action::None
            }
            TouchKind::Up => {
                self.press.up();
                let pressed = self.pressed.take();
                let swipe = SWIPE_DP * self.density;
                match self.pager.touch_up(t.x, t.y, t.t) {
                    Gesture::Tap { x, y } => match (pressed, self.target_at(x, y)) {
                        (Some(Target::Search), Some(Target::Search)) => Action::OpenSearch,
                        (Some(Target::AllApps), Some(Target::AllApps)) => Action::OpenAppList,
                        (Some(p), Some(now)) if p == now => match (p, self.item_at(p)) {
                            (_, Some(Item::App(key))) => Action::Launch(key),
                            (Target::Cell { page, col, row }, Some(Item::Folder(_))) => {
                                self.mode = Mode::Folder(OpenFolder { page, col, row, renaming: None, pressed: None });
                                Action::None
                            }
                            _ => Action::None,
                        },
                        _ => Action::None,
                    },
                    Gesture::VerticalSwipe { dy } if dy > swipe => Action::OpenNotifications,
                    Gesture::VerticalSwipe { dy } if dy < -swipe => Action::OpenAppList,
                    _ => Action::None,
                }
            }
            TouchKind::Cancel => {
                self.press.cancel();
                self.pager.touch_cancel(t.t);
                self.pressed = None;
                Action::None
            }
        }
    }

    fn far(&self, (x0, y0): (f32, f32), x: f32, y: f32) -> bool {
        (x - x0).hypot(y - y0) > PRESS_SLOP_DP * self.density
    }

    fn touch_menu(&mut self, mut lift: Lift, finger: Finger, t: Touch) -> Action {
        match (t.kind, finger) {
            (TouchKind::Down, _) => {
                let pressed = self.menu_hit(&lift, t.x, t.y);
                self.mode = Mode::Menu { lift, finger: Finger::Tap { x0: t.x, y0: t.y, pressed } };
                Action::None
            }
            // Moving the finger that long-pressed, or one put back on the
            // lifted icon, picks the item up and drags it.
            (TouchKind::Move, Finger::Held { x0, y0 } | Finger::Tap { x0, y0, pressed: MenuHit::Icon }) if self.far((x0, y0), t.x, t.y) => {
                self.take_out(&mut lift);
                log::info!("drag start");
                self.mode = Mode::Drag { lift, x: t.x, y: t.y, edge: None };
                Action::None
            }
            (TouchKind::Up, Finger::Tap { pressed, .. }) => {
                let now = self.menu_hit(&lift, t.x, t.y);
                if now != pressed {
                    self.mode = Mode::Menu { lift, finger: Finger::Up };
                    return Action::None;
                }
                match now {
                    MenuHit::Bar(i) => self.bar_action(lift, i),
                    MenuHit::Row(i) => {
                        let s = lift.shortcuts.get(i).cloned();
                        self.restore(lift);
                        s.map_or(Action::None, Action::StartShortcut)
                    }
                    MenuHit::Icon | MenuHit::Outside => {
                        self.restore(lift);
                        Action::None
                    }
                }
            }
            (TouchKind::Up, _) => {
                self.mode = Mode::Menu { lift, finger: Finger::Up };
                Action::None
            }
            (TouchKind::Cancel, _) => {
                self.restore(lift);
                Action::None
            }
            (TouchKind::Move, finger) => {
                self.mode = Mode::Menu { lift, finger };
                Action::None
            }
        }
    }

    fn touch_drag(&mut self, lift: Lift, x: f32, y: f32, edge: Option<(i8, Instant)>, t: Touch) -> Action {
        match t.kind {
            TouchKind::Move | TouchKind::Down => {
                let (m, ..) = self.metrics.expect("metrics before touches");
                let zone = EDGE_DP * self.density;
                let side: i8 = if t.x < zone { -1 } else if t.x > m.screen_w - zone { 1 } else { 0 };
                let edge = match (side, edge) {
                    (0, _) => None,
                    (s, Some((was, since))) if s == was => Some((was, since)),
                    (s, _) => Some((s, t.t)),
                };
                self.mode = Mode::Drag { lift, x: t.x, y: t.y, edge };
                Action::None
            }
            TouchKind::Up => self.drop(lift, t.x, t.y),
            TouchKind::Cancel => {
                let _ = (x, y);
                self.restore(lift);
                Action::None
            }
        }
    }

    fn touch_space(&mut self, x: f32, y: f32, finger: Finger, t: Touch) -> Action {
        let popup = self.space_popup(x, y);
        let hit = |tx: f32, ty: f32| match menu::popup_hit(popup, 2, tx, ty, self.density) {
            Some(i) => MenuHit::Row(i),
            None => MenuHit::Outside,
        };
        match (t.kind, finger) {
            (TouchKind::Down, _) => {
                self.mode = Mode::Space { x, y, finger: Finger::Tap { x0: t.x, y0: t.y, pressed: hit(t.x, t.y) } };
                Action::None
            }
            (TouchKind::Up, Finger::Tap { pressed, .. }) if pressed == hit(t.x, t.y) => match pressed {
                MenuHit::Row(0) => Action::PickWallpaper,
                MenuHit::Row(_) => Action::HomeSettings,
                _ => Action::None,
            },
            (TouchKind::Cancel, _) => Action::None,
            (TouchKind::Up, _) => {
                self.mode = Mode::Space { x, y, finger: Finger::Up };
                Action::None
            }
            (TouchKind::Move, finger) => {
                self.mode = Mode::Space { x, y, finger };
                Action::None
            }
        }
    }

    fn folder_hit(&self, f: &OpenFolder, x: f32, y: f32) -> Option<FolderHit> {
        let Some(Item::Folder(folder)) = self.layout.slot(f.page, f.col, f.row) else { return None };
        let panel = self.panel(folder.apps.len());
        Some(if let Some(i) = panel.hit(x, y, folder.apps.len()) {
            FolderHit::App(i)
        } else if inside(panel.name, x, y) {
            FolderHit::Name
        } else if inside(panel.panel, x, y) {
            FolderHit::Panel
        } else {
            FolderHit::Outside
        })
    }

    fn touch_folder(&mut self, mut f: OpenFolder, t: Touch) -> Action {
        let Some(hit) = self.folder_hit(&f, t.x, t.y) else {
            // The folder is gone (its apps were uninstalled).
            return Action::None;
        };
        match t.kind {
            TouchKind::Down => {
                f.pressed = Some(hit);
                match hit {
                    FolderHit::App(i) if f.renaming.is_none() => {
                        self.press.down(t.x, t.y, t.t);
                        self.press_at = Some((PressAt::FolderApp(i), t.x, t.y));
                    }
                    _ => self.press.cancel(),
                }
            }
            TouchKind::Move => {
                self.press.moved(t.x, t.y);
                if f.pressed != Some(hit) {
                    f.pressed = None;
                }
            }
            TouchKind::Cancel => {
                self.press.cancel();
                f.pressed = None;
            }
            TouchKind::Up => {
                self.press.up();
                let pressed = f.pressed.take();
                if pressed == Some(hit) {
                    match (hit, f.renaming.is_some()) {
                        // Typing a name: any tap outside the name keeps it.
                        (FolderHit::App(_) | FolderHit::Panel | FolderHit::Outside, true) => {
                            self.save_name(&f);
                            f.renaming = None;
                            self.mode = Mode::Folder(f);
                            return Action::EndRename;
                        }
                        (FolderHit::App(i), false) => {
                            if let Some(Item::Folder(folder)) = self.layout.slot(f.page, f.col, f.row) {
                                return folder.apps.get(i).cloned().map_or(Action::None, Action::Launch);
                            }
                        }
                        (FolderHit::Name, false) => {
                            f.renaming = Some(String::new());
                            self.mode = Mode::Folder(f);
                            return Action::StartRename;
                        }
                        (FolderHit::Outside, false) => return Action::None,
                        _ => {}
                    }
                }
            }
        }
        self.mode = Mode::Folder(f);
        Action::None
    }

    /// Text with a dark shadow under it so it reads on light wallpapers.
    #[allow(clippy::too_many_arguments)]
    fn shadowed(&self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, s: &str, x: f32, baseline: f32, em: f32, spacing: f32, max_w: f32, color: Rgba, weight: Weight) -> f32 {
        let d = self.density;
        r.text_spaced(gl, batch, s, x, baseline + 1.0 * d, em, spacing, max_w, Rgba::hex(BLACK).with_alpha(0.5), weight);
        r.text_spaced(gl, batch, s, x, baseline, em, spacing, max_w, color, weight)
    }

    /// Build the frame. Returns true while something is still moving.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, icons: &mut Icons, screen: (i32, i32), insets: Insets, now: Instant) -> bool {
        let m = self.metrics_for(screen, insets);
        let d = self.density;
        let size = m.icon.round() as u32;
        let m_icon = size as f32;
        icons.set_size(size);
        let white = Rgba::hex(WHITE);
        let accent = Rgba::hex(ACCENT);

        // Clock: bold hours, light minutes in the accent colour, centred.
        let (hours, minutes, date) = self.clock.clone();
        let em = CLOCK_SP * d;
        let tight = -2.0 * d;
        let hw = r.measure_spaced(&hours, em, tight, Weight::Bold);
        let mw = r.measure_spaced(&minutes, em, tight, Weight::Light);
        let x0 = (m.screen_w - hw - mw - tight) / 2.0;
        self.shadowed(gl, r, batch, &hours, x0, m.clock_baseline, em, tight, m.screen_w, white, Weight::Bold);
        self.shadowed(gl, r, batch, &minutes, x0 + hw + tight, m.clock_baseline, em, tight, m.screen_w, accent, Weight::Light);
        let em = DATE_SP * d;
        let spacing = 1.5 * d;
        let dw = r.measure_spaced(&date, em, spacing, Weight::Regular).min(m.screen_w - 32.0 * d);
        self.shadowed(gl, r, batch, &date, (m.screen_w - dw) / 2.0, m.date_baseline, em, spacing, m.screen_w - 32.0 * d, Rgba::hex(holo::DATE), Weight::Regular);

        // Pages: the current one, plus its neighbour while swiping.
        let offset = self.pager.offset(now);
        let label_em = LABEL_SP * d;
        let label_ascent = r.ascent(label_em);
        let label_max = m.cell_w - 8.0 * d;
        for page in 0..self.layout.pages.len() {
            let shift = page as f32 * m.screen_w - offset;
            if shift.abs() >= m.screen_w {
                continue;
            }
            for row in 0..m.rows {
                for col in 0..m.cols {
                    let Some(item) = self.layout.slot(page, col, row).cloned() else { continue };
                    let ir = m.icon_rect(col, row);
                    let ir = (ir.0 + shift, ir.1, m_icon, m_icon);
                    let pressed = self.pressed == Some(Target::Cell { page, col, row });
                    let label = match &item {
                        Item::App(key) => {
                            icons.draw(gl, r, batch, key, (ir.0, ir.1, m_icon), pressed, 1.0);
                            self.labels.get(key).cloned().unwrap_or_default()
                        }
                        Item::Folder(f) => {
                            folder::draw_icon(gl, r, batch, icons, f, (ir.0, ir.1, m_icon), pressed, d);
                            f.name.clone()
                        }
                    };
                    if !label.is_empty() {
                        let lw = r.measure(&label, label_em, label_max, Weight::Regular);
                        let cx = ir.0 + m_icon / 2.0;
                        let baseline = ir.1 + m_icon + m.label_gap + label_ascent;
                        self.shadowed(gl, r, batch, &label, cx - lw / 2.0, baseline, label_em, 0.0, label_max, white, Weight::Regular);
                    }
                }
            }
        }

        // Page bar: a thin track with the current page's share lit up.
        let pages = self.layout.pages.len();
        if pages > 1 {
            let (bx, by, bw, bh) = m.page_bar;
            batch.rect(Rect::new(bx, by, bw, bh), white.with_alpha(0.14), 0.0);
            let thumb = bw / pages as f32;
            let at = (offset / m.screen_w).clamp(0.0, (pages - 1) as f32);
            batch.rect(Rect::new(bx + at * thumb, by, thumb, bh), accent, 0.0);
        }

        // Dock: a line across the top, two apps, the all-apps button, two apps.
        batch.rect(Rect::new(0.0, m.dock_top, m.screen_w, 1.0 * d), white.with_alpha(0.18), 0.0);
        for i in 0..DOCK_SLOTS {
            if i == ALL_APPS_SLOT {
                continue;
            }
            let Some(key) = self.layout.dock[i as usize].clone() else { continue };
            let pressed = self.pressed == Some(Target::Dock(i));
            let ir = m.dock_icon_rect(i);
            icons.draw(gl, r, batch, &key, (ir.0, ir.1, ir.2), pressed, 1.0);
        }
        let (ax, ay, aw, _) = m.all_apps_rect();
        let (cx, cy, rad) = (ax + aw / 2.0, ay + aw / 2.0, aw / 2.0);
        let dim = if self.pressed == Some(Target::AllApps) { 0.6 } else { 1.0 };
        batch.circle(cx, cy, rad, Rgba::hex(BLACK).with_alpha(0.4));
        batch.outline(Rect::new(ax, ay, aw, aw), white.with_alpha(0.9 * dim), rad, 2.0 * d);
        let (sq_w, sq_h, gap) = (6.0 * d, 7.5 * d, 3.0 * d);
        for row in 0..2 {
            for col in 0..3 {
                let x = cx - 12.0 * d + col as f32 * (sq_w + gap);
                let y = cy - 9.0 * d + row as f32 * (sq_h + gap);
                batch.rect(Rect::new(x, y, sq_w, sq_h), white.with_alpha(dim), 0.0);
            }
        }

        // Search bar along the bottom.
        let (sx, sy, sw, sh) = m.search;
        let fill = if self.pressed == Some(Target::Search) { 0.18 } else { 0.10 };
        batch.rect(Rect::new(sx, sy, sw, sh), white.with_alpha(fill), 2.0 * d);
        let hint = Rgba::hex(holo::HINT);
        holo::magnifier(batch, sx + 16.0 * d + 11.0 * d, sy + sh / 2.0, d, hint);
        let em = 17.0 * d;
        let tx = sx + 16.0 * d + 22.0 * d + 14.0 * d;
        r.text(gl, batch, "Search apps", tx, sy + sh / 2.0 + r.ascent(em) / 2.0 - 1.0 * d, em, sx + sw - tx - 16.0 * d, hint, Weight::Light);

        self.draw_overlays(gl, r, batch, icons, &m, offset);
        self.pager.animating(now) || self.pager.dragging_pages()
    }

    /// An item picked up: an accent ring and glow around it.
    #[allow(clippy::too_many_arguments)]
    fn draw_lifted(&self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, icons: &mut Icons, item: &Item, (x, y, s): (f32, f32, f32)) {
        let d = self.density;
        let accent = Rgba::hex(ACCENT);
        let radius = match item {
            Item::App(_) => s * 0.18,
            Item::Folder(_) => s / 2.0,
        };
        batch.glow(Rect::new(x, y, s, s), accent.with_alpha(0.6), radius, 20.0 * d);
        let ring = 3.0 * d;
        batch.outline(Rect::new(x - ring, y - ring, s + 2.0 * ring, s + 2.0 * ring), accent, radius + ring, ring);
        match item {
            Item::App(key) => icons.draw(gl, r, batch, key, (x, y, s), false, 1.0),
            Item::Folder(f) => folder::draw_icon(gl, r, batch, icons, f, (x, y, s), false, d),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_overlays(&self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, icons: &mut Icons, m: &HomeMetrics, offset: f32) {
        let d = self.density;
        let dim = |batch: &mut Batch| batch.rect(Rect::new(0.0, 0.0, m.screen_w, m.screen_h), Rgba::hex(BLACK).with_alpha(0.55), 0.0);
        match &self.mode {
            Mode::Normal => {}
            Mode::Menu { lift, finger } => {
                dim(batch);
                let (labels, enabled) = Self::bar_labels(lift);
                menu::draw_top_bar(gl, r, batch, self.top_bar(), labels, enabled, None, d);
                self.draw_lifted(gl, r, batch, icons, &lift.item, lift.origin);
                if let Some(p) = self.popup(lift) {
                    let names: Vec<String> = lift.shortcuts.iter().map(|s| s.label.clone()).collect();
                    let pressed = match finger {
                        Finger::Tap { pressed: MenuHit::Row(i), .. } => Some(*i),
                        _ => None,
                    };
                    menu::draw_popup(gl, r, batch, p, &names, pressed, false, d);
                }
            }
            Mode::Drag { lift, x, y, .. } => {
                // A faint outline where it would land.
                let accent = Rgba::hex(ACCENT);
                match self.drop_pos(*x, *y) {
                    Some(Pos::Cell { page, col, row }) => {
                        let (cx, cy, cw, ch) = m.cell_rect(col, row);
                        let shift = page as f32 * m.screen_w - offset;
                        batch.outline(Rect::new(cx + shift + 4.0 * d, cy + 2.0 * d, cw - 8.0 * d, ch - 4.0 * d), accent.with_alpha(0.5), 4.0 * d, 2.0 * d);
                    }
                    Some(Pos::Dock(i)) if i != ALL_APPS_SLOT => {
                        let (ix, iy, s, _) = m.dock_icon_rect(i);
                        let pad = 6.0 * d;
                        batch.outline(Rect::new(ix - pad, iy - pad, s + 2.0 * pad, s + 2.0 * pad), accent.with_alpha(0.5), 4.0 * d, 2.0 * d);
                    }
                    _ => {}
                }
                let bar = self.top_bar();
                let (labels, enabled) = Self::bar_labels(lift);
                menu::draw_top_bar(gl, r, batch, bar, labels, enabled, menu::top_bar_hit(bar, *x, *y), d);
                let s = lift.origin.2 * 1.1;
                self.draw_lifted(gl, r, batch, icons, &lift.item, (x - s / 2.0, y - s / 2.0, s));
            }
            Mode::Space { x, y, finger } => {
                let pressed = match finger {
                    Finger::Tap { pressed: MenuHit::Row(i), .. } => Some(*i),
                    _ => None,
                };
                let labels = ["WALLPAPERS".to_string(), "HOME SETTINGS".to_string()];
                menu::draw_popup(gl, r, batch, self.space_popup(*x, *y), &labels, pressed, true, d);
            }
            Mode::Folder(f) => {
                let Some(Item::Folder(folder)) = self.layout.slot(f.page, f.col, f.row) else { return };
                dim(batch);
                let pressed = match f.pressed {
                    Some(FolderHit::App(i)) => Some(i),
                    _ => None,
                };
                self.panel(folder.apps.len()).draw(gl, r, batch, icons, folder, &self.labels, f.renaming.as_deref(), pressed, d);
            }
        }
    }
}

/// The app list without hidden apps.
fn visible_records(all: &[AppRecord], layout: &HomeLayout) -> Vec<AppRecord> {
    all.iter().filter(|r| !layout.is_hidden(&r.key)).cloned().collect()
}

/// For logs: a package name, or "a folder of N" (never a folder's name).
fn item_name(item: &Item) -> String {
    match item {
        Item::App(k) => k.package.clone(),
        Item::Folder(f) => format!("a folder of {}", f.apps.len()),
    }
}

