//! What's placed where on the home screen (apps, folders, the dock), which
//! apps are hidden, and the small text file it's all saved in.

use std::collections::{BTreeSet, HashSet};

use crate::geometry::{ALL_APPS_SLOT, DOCK_SLOTS};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppKey {
    pub package: String,
    pub activity: String,
}

impl AppKey {
    pub fn new(package: &str, activity: &str) -> AppKey {
        AppKey { package: package.to_string(), activity: activity.to_string() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub name: String,
    pub apps: Vec<AppKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    App(AppKey),
    Folder(Folder),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pos {
    Cell { page: usize, col: u32, row: u32 },
    Dock(u32),
}

pub const DEFAULT_FOLDER_NAME: &str = "Folder";

#[derive(Clone, Debug, PartialEq)]
pub struct HomeLayout {
    pub cols: u32,
    pub rows: u32,
    /// Row-major, `cols * rows` slots per page.
    pub pages: Vec<Vec<Option<Item>>>,
    pub dock: [Option<AppKey>; DOCK_SLOTS as usize],
    /// Left out of the app list, search and Home.
    pub hidden: BTreeSet<AppKey>,
}

impl HomeLayout {
    pub fn new(cols: u32, rows: u32) -> HomeLayout {
        HomeLayout {
            cols,
            rows,
            pages: vec![Self::empty_page(cols, rows)],
            dock: Default::default(),
            hidden: BTreeSet::new(),
        }
    }

    fn empty_page(cols: u32, rows: u32) -> Vec<Option<Item>> {
        vec![None; (cols * rows) as usize]
    }

    fn index(&self, col: u32, row: u32) -> usize {
        (row * self.cols + col) as usize
    }

    fn in_grid(&self, col: u32, row: u32) -> bool {
        col < self.cols && row < self.rows
    }

    pub fn slot(&self, page: usize, col: u32, row: u32) -> Option<&Item> {
        if !self.in_grid(col, row) {
            return None;
        }
        self.pages.get(page)?.get(self.index(col, row))?.as_ref()
    }

    fn slot_mut(&mut self, page: usize, col: u32, row: u32) -> Option<&mut Option<Item>> {
        if !self.in_grid(col, row) {
            return None;
        }
        let i = self.index(col, row);
        self.pages.get_mut(page)?.get_mut(i)
    }

    pub fn place(&mut self, page: usize, col: u32, row: u32, app: AppKey) {
        self.place_item(page, col, row, Item::App(app));
    }

    pub fn place_item(&mut self, page: usize, col: u32, row: u32, item: Item) {
        if !self.in_grid(col, row) {
            return;
        }
        self.ensure_page(page);
        let i = self.index(col, row);
        self.pages[page][i] = Some(item);
    }

    pub fn item_at(&self, pos: Pos) -> Option<Item> {
        match pos {
            Pos::Cell { page, col, row } => self.slot(page, col, row).cloned(),
            Pos::Dock(i) => self.dock.get(i as usize)?.clone().map(Item::App),
        }
    }

    /// Lifts whatever is at `pos` out of the layout.
    pub fn take(&mut self, pos: Pos) -> Option<Item> {
        match pos {
            Pos::Cell { page, col, row } => self.slot_mut(page, col, row)?.take(),
            Pos::Dock(i) => self.dock.get_mut(i as usize)?.take().map(Item::App),
        }
    }

    /// Puts `item` at `pos`: an empty cell takes it, an app dropped on an
    /// app makes a folder, an app dropped on a folder joins it, an empty
    /// dock slot (not the middle one) takes an app. Anything else hands the
    /// item back so the caller can return it where it came from.
    pub fn drop_at(&mut self, pos: Pos, item: Item) -> Result<(), Item> {
        match pos {
            Pos::Dock(i) => {
                let i = i as usize;
                if i == ALL_APPS_SLOT as usize || i >= self.dock.len() || self.dock[i].is_some() {
                    return Err(item);
                }
                match item {
                    Item::App(a) => {
                        self.dock[i] = Some(a);
                        Ok(())
                    }
                    folder => Err(folder),
                }
            }
            Pos::Cell { page, col, row } => {
                if !self.in_grid(col, row) || page > 64 {
                    return Err(item);
                }
                self.ensure_page(page);
                let slot = self.slot_mut(page, col, row).expect("page ensured");
                match (slot.as_mut(), item) {
                    (None, item) => {
                        *slot = Some(item);
                        Ok(())
                    }
                    (Some(Item::App(here)), Item::App(a)) => {
                        if *here != a {
                            let first = here.clone();
                            *slot = Some(Item::Folder(Folder { name: DEFAULT_FOLDER_NAME.into(), apps: vec![first, a] }));
                        }
                        Ok(())
                    }
                    (Some(Item::Folder(f)), Item::App(a)) => {
                        if !f.apps.contains(&a) {
                            f.apps.push(a);
                        }
                        Ok(())
                    }
                    (Some(_), item) => Err(item),
                }
            }
        }
    }

    /// Takes one app out of the folder at that cell. A folder left with
    /// one app turns back into that app.
    pub fn take_from_folder(&mut self, page: usize, col: u32, row: u32, index: usize) -> Option<AppKey> {
        let slot = self.slot_mut(page, col, row)?;
        let Some(Item::Folder(f)) = slot.as_mut() else { return None };
        if index >= f.apps.len() {
            return None;
        }
        let app = f.apps.remove(index);
        normalize(slot);
        Some(app)
    }

    pub fn rename_folder(&mut self, page: usize, col: u32, row: u32, name: &str) {
        if let Some(Some(Item::Folder(f))) = self.slot_mut(page, col, row) {
            let name = name.trim();
            f.name = if name.is_empty() { DEFAULT_FOLDER_NAME.to_string() } else { name.to_string() };
        }
    }

    pub fn hide(&mut self, app: &AppKey) {
        self.hidden.insert(app.clone());
        self.retain_apps(|a| a != app);
    }

    pub fn unhide(&mut self, app: &AppKey) {
        self.hidden.remove(app);
    }

    pub fn is_hidden(&self, app: &AppKey) -> bool {
        self.hidden.contains(app)
    }

    pub fn ensure_page(&mut self, page: usize) {
        while self.pages.len() <= page {
            self.pages.push(Self::empty_page(self.cols, self.rows));
        }
    }

    /// Drops empty pages at the end, always keeping the first. Empty pages
    /// in the middle stay (the owner may be keeping them on purpose).
    pub fn trim_empty_pages(&mut self) -> bool {
        let mut changed = false;
        while self.pages.len() > 1 && self.pages.last().is_some_and(|p| p.iter().all(|s| s.is_none())) {
            self.pages.pop();
            changed = true;
        }
        changed
    }

    pub fn contains(&self, app: &AppKey) -> bool {
        self.dock.iter().flatten().any(|a| a == app)
            || self.pages.iter().flatten().flatten().any(|item| match item {
                Item::App(a) => a == app,
                Item::Folder(f) => f.apps.contains(app),
            })
    }

    /// Removes every app for which `keep` says no, from the grid, folders
    /// and dock. Folders left with one app become it; empty ones vanish.
    fn retain_apps(&mut self, keep: impl Fn(&AppKey) -> bool) -> bool {
        let mut changed = false;
        for slot in self.dock.iter_mut() {
            if slot.as_ref().is_some_and(|a| !keep(a)) {
                *slot = None;
                changed = true;
            }
        }
        for slot in self.pages.iter_mut().flatten() {
            match slot {
                Some(Item::App(a)) if !keep(a) => {
                    *slot = None;
                    changed = true;
                }
                Some(Item::Folder(f)) => {
                    let before = f.apps.len();
                    f.apps.retain(|a| keep(a));
                    if f.apps.len() != before {
                        changed = true;
                        normalize(slot);
                    }
                }
                _ => {}
            }
        }
        changed
    }

    /// Empties every slot whose app is no longer installed (folder members
    /// included), forgets hidden apps that are gone, and drops trailing
    /// empty pages. Returns true when anything changed.
    pub fn remove_missing(&mut self, installed: &HashSet<AppKey>) -> bool {
        let mut changed = self.retain_apps(|a| installed.contains(a));
        let before = self.hidden.len();
        self.hidden.retain(|a| installed.contains(a));
        changed |= self.hidden.len() != before;
        changed |= self.trim_empty_pages();
        changed
    }

    /// Change the grid size without losing anything; see the tests.
    ///
    /// Items that still fit keep their place. The rest move, in order, to the
    /// first free slot on their own page or a later one (adding pages). An
    /// app in the dock's middle slot (the all-apps button) moves to a free
    /// dock slot, else to the first free grid slot.
    pub fn reflow(&mut self, cols: u32, rows: u32) -> bool {
        let mut changed = false;
        if (cols, rows) != (self.cols, self.rows) {
            let old_cols = self.cols;
            let old = std::mem::take(&mut self.pages);
            self.cols = cols;
            self.rows = rows;
            let mut overflow: Vec<(usize, Item)> = Vec::new();
            for (p, page) in old.into_iter().enumerate() {
                self.pages.push(Self::empty_page(cols, rows));
                for (i, item) in page.into_iter().enumerate() {
                    let Some(item) = item else { continue };
                    let (c, r) = (i as u32 % old_cols, i as u32 / old_cols);
                    if c < cols && r < rows {
                        let at = self.index(c, r);
                        self.pages[p][at] = Some(item);
                    } else {
                        overflow.push((p, item));
                    }
                }
            }
            if self.pages.is_empty() {
                self.pages.push(Self::empty_page(cols, rows));
            }
            for (from, item) in overflow {
                self.put_in_first_free(from, item);
            }
            changed = true;
        }
        let mid = ALL_APPS_SLOT as usize;
        if let Some(app) = self.dock[mid].take() {
            match (0..self.dock.len()).find(|&i| i != mid && self.dock[i].is_none()) {
                Some(free) => self.dock[free] = Some(app),
                None => self.put_in_first_free(0, Item::App(app)),
            }
            changed = true;
        }
        changed
    }

    pub fn put_in_first_free(&mut self, from_page: usize, item: Item) {
        let mut p = from_page;
        loop {
            self.ensure_page(p);
            if let Some(i) = self.pages[p].iter().position(|s| s.is_none()) {
                self.pages[p][i] = Some(item);
                return;
            }
            p += 1;
        }
    }

    /// Puts each newly installed app on Home, in the first free slot from
    /// the first page, unless it's hidden or already there. True if any went.
    pub fn add_new_apps(&mut self, apps: &[AppKey]) -> bool {
        let mut added = false;
        for app in apps {
            if !self.is_hidden(app) && !self.contains(app) {
                self.put_in_first_free(0, Item::App(app.clone()));
                added = true;
            }
        }
        added
    }

    pub fn to_text(&self) -> String {
        let mut out = format!("v1\ngrid {} {}\n", self.cols, self.rows);
        for (i, app) in self.dock.iter().enumerate() {
            if let Some(a) = app {
                out.push_str(&format!("dock {i} {}\n", key_text(a)));
            }
        }
        for (p, page) in self.pages.iter().enumerate() {
            for (i, item) in page.iter().enumerate() {
                let (col, row) = (i as u32 % self.cols, i as u32 / self.cols);
                match item {
                    Some(Item::App(a)) => out.push_str(&format!("slot {p} {col} {row} {}\n", key_text(a))),
                    Some(Item::Folder(f)) => {
                        out.push_str(&format!("folder {p} {col} {row} {}\n", escape_name(&f.name)));
                        for a in &f.apps {
                            out.push_str(&format!("in {p} {col} {row} {}\n", key_text(a)));
                        }
                    }
                    None => {}
                }
            }
        }
        for a in &self.hidden {
            out.push_str(&format!("hidden {}\n", key_text(a)));
        }
        out
    }

    pub fn from_text(s: &str) -> Option<HomeLayout> {
        let mut lines = s.lines();
        if lines.next()?.trim() != "v1" {
            return None;
        }
        let mut layout: Option<HomeLayout> = None;
        for line in lines {
            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts.first() {
                Some(&"grid") => {
                    let [_, c, r] = parts[..] else { return None };
                    let (cols, rows) = (c.parse().ok()?, r.parse().ok()?);
                    if cols == 0 || rows == 0 || cols > 12 || rows > 12 {
                        return None;
                    }
                    layout = Some(HomeLayout::new(cols, rows));
                }
                Some(&"dock") => {
                    let l = layout.as_mut()?;
                    let [_, i, app] = parts[..] else { return None };
                    let i: usize = i.parse().ok()?;
                    *l.dock.get_mut(i)? = Some(parse_key(app)?);
                }
                Some(&"slot") => {
                    let l = layout.as_mut()?;
                    let [_, p, c, r, app] = parts[..] else { return None };
                    let (p, c, r) = parse_cell(l, p, c, r)?;
                    l.place(p, c, r, parse_key(app)?);
                }
                Some(&"folder") => {
                    let l = layout.as_mut()?;
                    let [_, p, c, r, name] = parts[..] else { return None };
                    let (p, c, r) = parse_cell(l, p, c, r)?;
                    let name = unescape_name(name)?;
                    l.place_item(p, c, r, Item::Folder(Folder { name, apps: Vec::new() }));
                }
                Some(&"in") => {
                    let l = layout.as_mut()?;
                    let [_, p, c, r, app] = parts[..] else { return None };
                    let (p, c, r) = parse_cell(l, p, c, r)?;
                    let app = parse_key(app)?;
                    if let Some(Some(Item::Folder(f))) = l.slot_mut(p, c, r) {
                        if !f.apps.contains(&app) {
                            f.apps.push(app);
                        }
                    }
                }
                Some(&"hidden") => {
                    let l = layout.as_mut()?;
                    let [_, app] = parts[..] else { return None };
                    l.hidden.insert(parse_key(app)?);
                }
                _ => {}
            }
        }
        let mut layout = layout?;
        for slot in layout.pages.iter_mut().flatten() {
            normalize(slot);
        }
        Some(layout)
    }
}

/// A folder with one app becomes that app; an empty one disappears.
fn normalize(slot: &mut Option<Item>) {
    if let Some(Item::Folder(f)) = slot {
        match f.apps.len() {
            0 => *slot = None,
            1 => *slot = Some(Item::App(f.apps.remove(0))),
            _ => {}
        }
    }
}

fn parse_cell(l: &HomeLayout, p: &str, c: &str, r: &str) -> Option<(usize, u32, u32)> {
    let (p, c, r): (usize, u32, u32) = (p.parse().ok()?, c.parse().ok()?, r.parse().ok()?);
    if c >= l.cols || r >= l.rows || p > 64 {
        return None;
    }
    Some((p, c, r))
}

fn key_text(a: &AppKey) -> String {
    format!("{}/{}", a.package, a.activity)
}

fn parse_key(s: &str) -> Option<AppKey> {
    let (package, activity) = s.split_once('/')?;
    if package.is_empty() || activity.is_empty() {
        return None;
    }
    Some(AppKey::new(package, activity))
}

/// Folder names go on one line as one word: spaces become `\s`, and tabs,
/// new lines and backslashes are escaped as in the app list file.
fn escape_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ' ' => out.push_str("\\s"),
            c if c.is_whitespace() => out.push_str("\\s"),
            c => out.push(c),
        }
    }
    if out.is_empty() {
        out.push_str(DEFAULT_FOLDER_NAME);
    }
    out
}

fn unescape_name(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            '\\' => '\\',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            's' => ' ',
            _ => return None,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(p: &str) -> AppKey {
        AppKey::new(p, &format!("{p}.Main"))
    }

    fn app(p: &str) -> Item {
        Item::App(k(p))
    }

    fn sample() -> HomeLayout {
        let mut l = HomeLayout::new(4, 5);
        l.place(0, 1, 2, k("com.a"));
        l.place(0, 3, 4, k("com.b"));
        l.place(1, 0, 0, k("com.c"));
        l.dock[0] = Some(k("com.dial"));
        l.dock[4] = Some(k("com.settings"));
        l
    }

    fn all_apps(l: &HomeLayout) -> Vec<AppKey> {
        let mut v: Vec<AppKey> = l.dock.iter().flatten().cloned().collect();
        for item in l.pages.iter().flatten().flatten() {
            match item {
                Item::App(a) => v.push(a.clone()),
                Item::Folder(f) => v.extend(f.apps.iter().cloned()),
            }
        }
        v.sort();
        v
    }

    fn cell(page: usize, col: u32, row: u32) -> Pos {
        Pos::Cell { page, col, row }
    }

    #[test]
    fn app_on_app_makes_a_folder() {
        let mut l = HomeLayout::new(4, 4);
        l.place(0, 0, 0, k("a"));
        assert_eq!(l.drop_at(cell(0, 0, 0), Item::App(k("b"))), Ok(()));
        assert_eq!(l.slot(0, 0, 0), Some(&Item::Folder(Folder { name: "Folder".into(), apps: vec![k("a"), k("b")] })));
    }

    #[test]
    fn app_on_folder_joins_it_once() {
        let mut l = HomeLayout::new(4, 4);
        l.place(0, 0, 0, k("a"));
        l.drop_at(cell(0, 0, 0), Item::App(k("b"))).unwrap();
        l.drop_at(cell(0, 0, 0), Item::App(k("c"))).unwrap();
        l.drop_at(cell(0, 0, 0), Item::App(k("c"))).unwrap();
        let Some(Item::Folder(f)) = l.slot(0, 0, 0) else { panic!() };
        assert_eq!(f.apps, vec![k("a"), k("b"), k("c")]);
    }

    #[test]
    fn drops_that_cannot_land_come_back() {
        let mut l = HomeLayout::new(4, 4);
        l.place(0, 0, 0, k("a"));
        l.place(0, 1, 0, k("b"));
        l.drop_at(cell(0, 1, 0), Item::App(k("c"))).unwrap();
        let folder = l.take(cell(0, 1, 0)).unwrap();
        assert_eq!(l.drop_at(cell(0, 0, 0), folder.clone()), Err(folder.clone()));
        l.dock[0] = Some(k("d"));
        assert_eq!(l.drop_at(Pos::Dock(0), Item::App(k("e"))), Err(Item::App(k("e"))));
        assert_eq!(l.drop_at(Pos::Dock(2), Item::App(k("e"))), Err(Item::App(k("e"))));
        assert_eq!(l.drop_at(Pos::Dock(1), folder.clone()), Err(folder));
        assert_eq!(l.drop_at(Pos::Dock(1), Item::App(k("e"))), Ok(()));
        assert_eq!(l.dock[1], Some(k("e")));
    }

    #[test]
    fn take_and_drop_moves() {
        let mut l = HomeLayout::new(4, 4);
        l.dock[0] = Some(k("d"));
        let it = l.take(Pos::Dock(0)).unwrap();
        assert_eq!(l.dock[0], None);
        l.drop_at(cell(1, 2, 3), it).unwrap();
        assert_eq!(l.slot(1, 2, 3), Some(&app("d")));
        assert_eq!(l.pages.len(), 2);
        let it = l.take(cell(1, 2, 3)).unwrap();
        assert_eq!(l.item_at(cell(1, 2, 3)), None);
        l.drop_at(Pos::Dock(4), it).unwrap();
        assert_eq!(l.dock[4], Some(k("d")));
    }

    #[test]
    fn folders_and_hidden_round_trip_and_old_files_load() {
        let mut l = HomeLayout::new(4, 4);
        l.place(0, 0, 0, k("a"));
        l.drop_at(cell(0, 0, 0), Item::App(k("b"))).unwrap();
        l.rename_folder(0, 0, 0, "My\tstuff \\ here");
        l.place(0, 1, 0, k("c"));
        l.hide(&k("h"));
        let text = l.to_text();
        assert_eq!(HomeLayout::from_text(&text), Some(l));
        let old = "v1\ngrid 4 5\ndock 0 a/A\nslot 0 1 2 b/B\n";
        let old = HomeLayout::from_text(old).unwrap();
        assert_eq!(old.slot(0, 1, 2), Some(&Item::App(AppKey::new("b", "B"))));
        assert!(old.hidden.is_empty());
        assert!(HomeLayout::from_text("v1\ngrid 4 4\nin 0 0 0 a/A\n").unwrap().slot(0, 0, 0).is_none());
    }

    #[test]
    fn hide_removes_everywhere() {
        let mut l = HomeLayout::new(4, 4);
        l.place(0, 0, 0, k("x"));
        l.dock[0] = Some(k("x"));
        l.place(0, 1, 0, k("a"));
        l.drop_at(cell(0, 1, 0), Item::App(k("b"))).unwrap();
        l.drop_at(cell(0, 1, 0), Item::App(k("x"))).unwrap();
        l.hide(&k("x"));
        assert!(!l.contains(&k("x")));
        assert!(l.is_hidden(&k("x")));
        l.unhide(&k("x"));
        assert!(!l.is_hidden(&k("x")));
    }

    #[test]
    fn uninstalled_folder_member_leaves_and_one_app_folder_dissolves() {
        let mut l = HomeLayout::new(4, 4);
        l.place(0, 0, 0, k("a"));
        l.drop_at(cell(0, 0, 0), Item::App(k("b"))).unwrap();
        let installed: HashSet<AppKey> = [k("a")].into_iter().collect();
        assert!(l.remove_missing(&installed));
        assert_eq!(l.slot(0, 0, 0), Some(&app("a")));
    }

    #[test]
    fn take_from_folder_dissolves_at_one() {
        let mut l = HomeLayout::new(4, 4);
        l.place(0, 0, 0, k("a"));
        l.drop_at(cell(0, 0, 0), Item::App(k("b"))).unwrap();
        l.drop_at(cell(0, 0, 0), Item::App(k("c"))).unwrap();
        assert_eq!(l.take_from_folder(0, 0, 0, 1), Some(k("b")));
        assert_eq!(l.take_from_folder(0, 0, 0, 1), Some(k("c")));
        assert_eq!(l.slot(0, 0, 0), Some(&app("a")));
        assert_eq!(l.take_from_folder(0, 0, 0, 0), None);
    }

    #[test]
    fn new_apps_fill_free_slots_but_skip_hidden_and_placed() {
        let mut l = HomeLayout::new(2, 1);
        l.place(0, 0, 0, k("a"));
        l.hide(&k("h"));
        assert!(l.add_new_apps(&[k("a"), k("h"), k("n1"), k("n2")]));
        assert_eq!(l.slot(0, 1, 0), Some(&app("n1")));
        assert_eq!(l.slot(1, 0, 0), Some(&app("n2")));
        assert!(!l.contains(&k("h")));
        assert!(!l.add_new_apps(&[k("a"), k("n1")]));
    }

    #[test]
    fn trim_keeps_first_and_middle_pages() {
        let mut l = HomeLayout::new(4, 4);
        l.ensure_page(3);
        l.place(2, 0, 0, k("a"));
        assert_eq!(l.pages.len(), 4);
        assert!(l.trim_empty_pages());
        assert_eq!(l.pages.len(), 3);
        assert!(!l.trim_empty_pages());
    }

    #[test]
    fn reflow_to_fewer_rows_keeps_every_app() {
        let mut l = HomeLayout::new(4, 5);
        l.place(0, 0, 0, k("keep"));
        l.place(0, 1, 4, k("row4a"));
        l.place(0, 3, 4, k("row4b"));
        for i in 0..20 {
            l.place(1, i % 4, i / 4, k(&format!("p1-{i}")));
        }
        let before = all_apps(&l);
        assert!(l.reflow(4, 4));
        assert_eq!((l.cols, l.rows), (4, 4));
        assert!(l.pages.iter().all(|p| p.len() == 16));
        assert_eq!(all_apps(&l), before);
        assert_eq!(l.slot(0, 0, 0), Some(&app("keep")));
        // The two dropped apps take the first free slots on page 0.
        assert_eq!(l.slot(0, 1, 0), Some(&app("row4a")));
        assert_eq!(l.slot(0, 2, 0), Some(&app("row4b")));
        // Page 1's last row overflows onto a new page.
        assert_eq!(l.pages.len(), 3);
        assert_eq!(l.slot(1, 3, 3), Some(&app("p1-15")));
        assert!(!l.reflow(4, 4));
    }

    #[test]
    fn reflow_moves_an_app_out_of_the_all_apps_slot() {
        let mut l = HomeLayout::new(4, 4);
        l.dock[2] = Some(k("mid"));
        assert!(l.reflow(4, 4));
        assert_eq!(l.dock[2], None);
        assert_eq!(l.dock[0], Some(k("mid")));
    }

    #[test]
    fn reflow_finds_a_free_dock_slot_after_the_middle() {
        let mut l = HomeLayout::new(4, 4);
        l.dock[0] = Some(k("a"));
        l.dock[1] = Some(k("b"));
        l.dock[2] = Some(k("mid"));
        assert!(l.reflow(4, 4));
        assert_eq!(l.dock[3], Some(k("mid")));
    }

    #[test]
    fn reflow_puts_the_middle_app_on_the_grid_when_the_dock_is_full() {
        let mut l = HomeLayout::new(4, 4);
        for i in 0..5 {
            l.dock[i] = Some(k(&format!("d{i}")));
        }
        let before = all_apps(&l);
        assert!(l.reflow(4, 4));
        assert_eq!(l.dock[2], None);
        assert_eq!(l.slot(0, 0, 0), Some(&app("d2")));
        assert_eq!(all_apps(&l), before);
    }

    #[test]
    fn new_has_one_empty_page() {
        let l = HomeLayout::new(4, 5);
        assert_eq!(l.pages.len(), 1);
        assert_eq!(l.pages[0].len(), 20);
        assert!(l.pages[0].iter().all(|s| s.is_none()));
        assert!(l.dock.iter().all(|s| s.is_none()));
    }

    #[test]
    fn place_adds_pages_in_between() {
        let mut l = HomeLayout::new(4, 5);
        l.place(2, 1, 1, k("com.x"));
        assert_eq!(l.pages.len(), 3);
        assert_eq!(l.slot(2, 1, 1), Some(&app("com.x")));
        assert!(l.contains(&k("com.x")));
        assert!(!l.contains(&k("com.y")));
    }

    #[test]
    fn round_trip() {
        let l = sample();
        let text = l.to_text();
        assert_eq!(HomeLayout::from_text(&text), Some(l));
    }

    #[test]
    fn garbage_is_none() {
        assert_eq!(HomeLayout::from_text("garbage"), None);
        assert_eq!(HomeLayout::from_text(""), None);
        assert_eq!(HomeLayout::from_text("v1\ngrid four 5\n"), None);
        assert_eq!(HomeLayout::from_text("v1\ngrid 4 5\nslot 0 9 0 a/b\n"), None, "column outside grid");
        assert_eq!(HomeLayout::from_text("v1\ngrid 4 5\ndock 7 a/b\n"), None, "dock slot outside");
        assert_eq!(HomeLayout::from_text("v1\ngrid 4 5\nslot 0 1 1 nopackage\n"), None);
    }

    #[test]
    fn unknown_lines_are_ignored() {
        let mut text = sample().to_text();
        text.push_str("folder 0 1 1 something\n\n");
        assert_eq!(HomeLayout::from_text(&text), Some(sample()));
    }

    #[test]
    fn remove_missing_empties_gone_apps_once() {
        let mut l = sample();
        let installed: HashSet<AppKey> = [k("com.a"), k("com.c"), k("com.dial")].into_iter().collect();
        assert!(l.remove_missing(&installed));
        assert_eq!(l.slot(0, 3, 4), None);
        assert_eq!(l.dock[4], None);
        assert_eq!(l.slot(0, 1, 2), Some(&app("com.a")));
        assert!(!l.remove_missing(&installed));
    }

    #[test]
    fn trailing_empty_pages_are_dropped_but_one_stays() {
        let mut l = sample();
        let installed: HashSet<AppKey> = [k("com.a")].into_iter().collect();
        assert!(l.remove_missing(&installed));
        assert_eq!(l.pages.len(), 1);
        let mut l = sample();
        assert!(l.remove_missing(&HashSet::new()));
        assert_eq!(l.pages.len(), 1);
    }
}
