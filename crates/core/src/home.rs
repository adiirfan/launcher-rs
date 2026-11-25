//! What's placed where on the home screen, and the small text file it's saved in.

use std::collections::HashSet;

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
pub struct HomeLayout {
    pub cols: u32,
    pub rows: u32,
    /// Row-major, `cols * rows` slots per page.
    pub pages: Vec<Vec<Option<AppKey>>>,
    pub dock: [Option<AppKey>; DOCK_SLOTS as usize],
}

impl HomeLayout {
    pub fn new(cols: u32, rows: u32) -> HomeLayout {
        HomeLayout { cols, rows, pages: vec![Self::empty_page(cols, rows)], dock: Default::default() }
    }

    fn empty_page(cols: u32, rows: u32) -> Vec<Option<AppKey>> {
        vec![None; (cols * rows) as usize]
    }

    fn index(&self, col: u32, row: u32) -> usize {
        (row * self.cols + col) as usize
    }

    pub fn slot(&self, page: usize, col: u32, row: u32) -> Option<&AppKey> {
        self.pages.get(page)?.get(self.index(col, row))?.as_ref()
    }

    pub fn place(&mut self, page: usize, col: u32, row: u32, app: AppKey) {
        while self.pages.len() <= page {
            self.pages.push(Self::empty_page(self.cols, self.rows));
        }
        let i = self.index(col, row);
        self.pages[page][i] = Some(app);
    }

    pub fn contains(&self, app: &AppKey) -> bool {
        self.pages.iter().flatten().chain(self.dock.iter()).any(|s| s.as_ref() == Some(app))
    }

    /// Empties every slot whose app is no longer installed and drops
    /// trailing pages left empty. Returns true when anything changed.
    pub fn remove_missing(&mut self, installed: &HashSet<AppKey>) -> bool {
        let mut changed = false;
        for slot in self.pages.iter_mut().flatten().chain(self.dock.iter_mut()) {
            if slot.as_ref().is_some_and(|a| !installed.contains(a)) {
                *slot = None;
                changed = true;
            }
        }
        while self.pages.len() > 1 && self.pages.last().is_some_and(|p| p.iter().all(|s| s.is_none())) {
            self.pages.pop();
            changed = true;
        }
        changed
    }

    /// Change the grid size without losing an app; see the tests.
    ///
    /// Apps that still fit keep their place. The rest move, in order, to the
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
            let mut overflow: Vec<(usize, AppKey)> = Vec::new();
            for (p, page) in old.into_iter().enumerate() {
                self.pages.push(Self::empty_page(cols, rows));
                for (i, app) in page.into_iter().enumerate() {
                    let Some(app) = app else { continue };
                    let (c, r) = (i as u32 % old_cols, i as u32 / old_cols);
                    if c < cols && r < rows {
                        let at = self.index(c, r);
                        self.pages[p][at] = Some(app);
                    } else {
                        overflow.push((p, app));
                    }
                }
            }
            if self.pages.is_empty() {
                self.pages.push(Self::empty_page(cols, rows));
            }
            for (from, app) in overflow {
                self.put_in_first_free(from, app);
            }
            changed = true;
        }
        let mid = ALL_APPS_SLOT as usize;
        if let Some(app) = self.dock[mid].take() {
            match (0..self.dock.len()).find(|&i| i != mid && self.dock[i].is_none()) {
                Some(free) => self.dock[free] = Some(app),
                None => self.put_in_first_free(0, app),
            }
            changed = true;
        }
        changed
    }

    fn put_in_first_free(&mut self, from_page: usize, app: AppKey) {
        let mut p = from_page;
        loop {
            while self.pages.len() <= p {
                self.pages.push(Self::empty_page(self.cols, self.rows));
            }
            if let Some(i) = self.pages[p].iter().position(|s| s.is_none()) {
                self.pages[p][i] = Some(app);
                return;
            }
            p += 1;
        }
    }

    pub fn to_text(&self) -> String {
        let mut out = format!("v1\ngrid {} {}\n", self.cols, self.rows);
        for (i, app) in self.dock.iter().enumerate() {
            if let Some(a) = app {
                out.push_str(&format!("dock {i} {}/{}\n", a.package, a.activity));
            }
        }
        for (p, page) in self.pages.iter().enumerate() {
            for (i, app) in page.iter().enumerate() {
                if let Some(a) = app {
                    let (col, row) = (i as u32 % self.cols, i as u32 / self.cols);
                    out.push_str(&format!("slot {p} {col} {row} {}/{}\n", a.package, a.activity));
                }
            }
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
                    let (p, c, r): (usize, u32, u32) = (p.parse().ok()?, c.parse().ok()?, r.parse().ok()?);
                    if c >= l.cols || r >= l.rows || p > 64 {
                        return None;
                    }
                    l.place(p, c, r, parse_key(app)?);
                }
                _ => {}
            }
        }
        layout
    }
}

fn parse_key(s: &str) -> Option<AppKey> {
    let (package, activity) = s.split_once('/')?;
    if package.is_empty() || activity.is_empty() {
        return None;
    }
    Some(AppKey::new(package, activity))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(p: &str) -> AppKey {
        AppKey::new(p, &format!("{p}.Main"))
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
        let mut v: Vec<AppKey> = l.pages.iter().flatten().chain(l.dock.iter()).flatten().cloned().collect();
        v.sort();
        v
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
        assert_eq!(l.slot(0, 0, 0), Some(&k("keep")));
        // The two dropped apps take the first free slots on page 0.
        assert_eq!(l.slot(0, 1, 0), Some(&k("row4a")));
        assert_eq!(l.slot(0, 2, 0), Some(&k("row4b")));
        // Page 1's last row overflows onto a new page.
        assert_eq!(l.pages.len(), 3);
        assert_eq!(l.slot(1, 3, 3), Some(&k("p1-15")));
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
        assert_eq!(l.slot(0, 0, 0), Some(&k("d2")));
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
        assert_eq!(l.slot(2, 1, 1), Some(&k("com.x")));
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
        assert_eq!(l.slot(0, 1, 2), Some(&k("com.a")));
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
