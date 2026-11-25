//! Where everything sits on the Holo home screen, in pixels.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Cell { col: u32, row: u32 },
    Dock(u32),
    AllApps,
    Search,
}

pub const DOCK_SLOTS: u32 = 5;
/// The dock's middle slot holds the all-apps button.
pub const ALL_APPS_SLOT: u32 = 2;
pub const HOLO_COLS: u32 = 4;
pub const HOLO_ROWS: u32 = 4;
pub const CLOCK_SP: f32 = 84.0;
/// On a wide screen (a tablet, a foldable opened up) Home keeps to a centred
/// column this wide, so the grid, dock and search bar stay phone-sized.
pub const COLUMN_MAX_DP: f32 = 520.0;

#[derive(Clone, Copy, Debug)]
pub struct HomeMetrics {
    pub screen_w: f32,
    pub screen_h: f32,
    pub density: f32,
    pub cols: u32,
    pub rows: u32,
    /// Icon edge, at most 56 dp; smaller when the grid is short.
    pub icon: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    pub grid_left: f32,
    pub grid_top: f32,
    pub label_gap: f32,
    pub clock_baseline: f32,
    pub date_baseline: f32,
    /// The line under the date (next event or music).
    pub info_baseline: f32,
    /// The thin page bar: x, y, w, h.
    pub page_bar: (f32, f32, f32, f32),
    pub dock_top: f32,
    pub dock_h: f32,
    pub dock_left: f32,
    pub dock_slot_w: f32,
    /// Search bar: x, y, w, h.
    pub search: (f32, f32, f32, f32),
}

impl HomeMetrics {
    pub fn new(screen_w: f32, screen_h: f32, density: f32, insets: Insets, cols: u32, rows: u32, info_line: bool) -> HomeMetrics {
        let d = density;
        let inner_w = screen_w - insets.left - insets.right;
        let col_w = inner_w.min(COLUMN_MAX_DP * d);
        let left = insets.left + (inner_w - col_w) / 2.0;

        let search_h = 48.0 * d;
        let search = (left + 12.0 * d, screen_h - insets.bottom - 8.0 * d - search_h, col_w - 24.0 * d, search_h);
        let dock_h = 72.0 * d;
        let dock_top = search.1 - 8.0 * d - dock_h;
        let page_bar = (left + 24.0 * d, dock_top - 12.0 * d - 3.0 * d, col_w - 48.0 * d, 3.0 * d);

        let clock_baseline = insets.top + 56.0 * d + CLOCK_SP * d * 0.75;
        let date_baseline = clock_baseline + 26.0 * d;
        let info_baseline = date_baseline + 22.0 * d;
        // One more line under the date (next event or music) pushes the grid down.
        let grid_top = insets.top + 160.0 * d + if info_line { 24.0 * d } else { 0.0 };
        let grid_bottom = page_bar.1 - 8.0 * d;
        let cell_w = (col_w - 16.0 * d) / cols as f32;
        let cell_h = ((grid_bottom - grid_top) / rows as f32).max(0.0);
        // The label takes 6 dp of gap plus an 18 dp line; keep 4 dp spare.
        let icon = (56.0 * d).min(cell_h - 28.0 * d).max(0.0);

        HomeMetrics {
            screen_w,
            screen_h,
            density,
            cols,
            rows,
            icon,
            cell_w,
            cell_h,
            grid_left: left + 8.0 * d,
            grid_top,
            label_gap: 6.0 * d,
            clock_baseline,
            date_baseline,
            info_baseline,
            page_bar,
            dock_top,
            dock_h,
            dock_left: left + 16.0 * d,
            dock_slot_w: (col_w - 32.0 * d) / DOCK_SLOTS as f32,
            search,
        }
    }

    pub fn cell_rect(&self, col: u32, row: u32) -> (f32, f32, f32, f32) {
        (self.grid_left + col as f32 * self.cell_w, self.grid_top + row as f32 * self.cell_h, self.cell_w, self.cell_h)
    }

    pub fn icon_rect(&self, col: u32, row: u32) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.cell_rect(col, row);
        let group = self.icon + self.label_gap + 18.0 * self.density;
        (x + (w - self.icon) / 2.0, y + (h - group) / 2.0, self.icon, self.icon)
    }

    fn dock_centre(&self, i: u32) -> (f32, f32) {
        (self.dock_left + (i as f32 + 0.5) * self.dock_slot_w, self.dock_top + self.dock_h / 2.0)
    }

    pub fn dock_icon_rect(&self, i: u32) -> (f32, f32, f32, f32) {
        let s = self.icon.min(self.dock_h - 16.0 * self.density);
        let (cx, cy) = self.dock_centre(i);
        (cx - s / 2.0, cy - s / 2.0, s, s)
    }

    pub fn all_apps_rect(&self) -> (f32, f32, f32, f32) {
        let s = 46.0 * self.density;
        let (cx, cy) = self.dock_centre(ALL_APPS_SLOT);
        (cx - s / 2.0, cy - s / 2.0, s, s)
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let inside = |r: (f32, f32, f32, f32)| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3;
        if inside(self.search) {
            return Some(Hit::Search);
        }
        let dock = (self.dock_left, self.dock_top, self.dock_slot_w * DOCK_SLOTS as f32, self.dock_h);
        if inside(dock) {
            let i = (((x - self.dock_left) / self.dock_slot_w) as u32).min(DOCK_SLOTS - 1);
            return Some(if i == ALL_APPS_SLOT { Hit::AllApps } else { Hit::Dock(i) });
        }
        let grid = (self.grid_left, self.grid_top, self.cell_w * self.cols as f32, self.cell_h * self.rows as f32);
        if inside(grid) {
            let col = (((x - self.grid_left) / self.cell_w) as u32).min(self.cols - 1);
            let row = (((y - self.grid_top) / self.cell_h) as u32).min(self.rows - 1);
            return Some(Hit::Cell { col, row });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> HomeMetrics {
        let ins = Insets { top: 72.0, bottom: 144.0, left: 0.0, right: 0.0 };
        HomeMetrics::new(1080.0, 2160.0, 3.0, ins, HOLO_COLS, HOLO_ROWS, false)
    }

    fn centre(r: (f32, f32, f32, f32)) -> (f32, f32) {
        (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
    }

    #[test]
    fn wide_screen_keeps_home_in_a_centred_column() {
        // The Fold's inner screen, sideways: 2504 x 2256 at 3x.
        let ins = Insets { top: 72.0, bottom: 72.0, left: 0.0, right: 0.0 };
        let m = HomeMetrics::new(2504.0, 2256.0, 3.0, ins, HOLO_COLS, HOLO_ROWS, false);
        let mid = 1252.0;
        assert!((centre(m.search).0 - mid).abs() < 0.01);
        assert_eq!(m.search.2, (COLUMN_MAX_DP - 24.0) * 3.0);
        assert!((m.cell_w - (COLUMN_MAX_DP - 16.0) * 3.0 / 4.0).abs() < 0.01);
        assert!((m.grid_left + 2.0 * m.cell_w - mid).abs() < 0.01);
        assert!((centre(m.all_apps_rect()).0 - mid).abs() < 0.01);
        assert!((centre(m.page_bar).0 - mid).abs() < 0.01);
        assert_eq!(m.hit(centre(m.all_apps_rect()).0, centre(m.all_apps_rect()).1), Some(Hit::AllApps));
        assert_eq!(m.hit(m.grid_left - 10.0, m.grid_top + 10.0), None);
    }

    #[test]
    fn short_screen_fits_everything_between_the_bars() {
        let m = m();
        let (sx, sy, sw, sh) = m.search;
        assert!((sy + sh - (2160.0 - 144.0 - 24.0)).abs() < 0.01);
        assert_eq!((sx, sw, sh), (36.0, 1008.0, 144.0));
        assert!(m.dock_top + m.dock_h <= sy + 0.01);
        assert!(m.page_bar.1 + m.page_bar.3 <= m.dock_top + 0.01);
        assert!(m.grid_top + m.rows as f32 * m.cell_h <= m.page_bar.1 + 0.01);
        assert!(m.grid_top >= m.date_baseline);
        assert!(m.clock_baseline > 72.0 && m.date_baseline > m.clock_baseline);
        assert!(m.icon >= 40.0 * 3.0 && m.icon <= 56.0 * 3.0);
        assert!(m.icon + m.label_gap + 18.0 * 3.0 <= m.cell_h + 0.01);
    }

    #[test]
    fn info_line_moves_the_grid_down_and_it_still_fits() {
        let ins = Insets { top: 72.0, bottom: 144.0, left: 0.0, right: 0.0 };
        let plain = m();
        let with = HomeMetrics::new(1080.0, 2160.0, 3.0, ins, HOLO_COLS, HOLO_ROWS, true);
        assert!((with.grid_top - plain.grid_top - 24.0 * 3.0).abs() < 0.01);
        assert!((with.info_baseline - with.date_baseline - 22.0 * 3.0).abs() < 0.01);
        assert!(with.info_baseline < with.grid_top);
        assert!(with.grid_top + with.rows as f32 * with.cell_h <= with.page_bar.1 + 0.01);
        assert!(with.icon >= 40.0 * 3.0);
        assert!(with.icon + with.label_gap + 18.0 * 3.0 <= with.cell_h + 0.01);
    }

    #[test]
    fn tall_screen_gets_full_size_icons() {
        let ins = Insets { top: 72.0, bottom: 144.0, left: 0.0, right: 0.0 };
        let m = HomeMetrics::new(1080.0, 2400.0, 2.625, ins, 4, 4, false);
        assert_eq!(m.icon, 56.0 * 2.625);
    }

    #[test]
    fn page_bar_spans_the_width_minus_margins() {
        let m = m();
        assert_eq!(m.page_bar.0, 72.0);
        assert_eq!(m.page_bar.2, 1080.0 - 144.0);
        assert_eq!(m.page_bar.3, 9.0);
    }

    #[test]
    fn cells_tile_and_icons_sit_inside() {
        let m = m();
        let a = m.cell_rect(0, 0);
        let b = m.cell_rect(1, 0);
        assert!((a.0 + a.2 - b.0).abs() < 0.01);
        let c = m.cell_rect(2, 3);
        let i = m.icon_rect(2, 3);
        assert!(i.0 >= c.0 && i.0 + i.2 <= c.0 + c.2 && i.1 >= c.1 && i.1 + i.3 <= c.1 + c.3);
    }

    #[test]
    fn hits() {
        let m = m();
        let (x, y) = centre(m.icon_rect(2, 3));
        assert_eq!(m.hit(x, y), Some(Hit::Cell { col: 2, row: 3 }));
        let (x, y) = centre(m.dock_icon_rect(0));
        assert_eq!(m.hit(x, y), Some(Hit::Dock(0)));
        let (x, y) = centre(m.dock_icon_rect(4));
        assert_eq!(m.hit(x, y), Some(Hit::Dock(4)));
        let (x, y) = centre(m.all_apps_rect());
        assert_eq!(m.hit(x, y), Some(Hit::AllApps));
        let (x, y) = centre(m.search);
        assert_eq!(m.hit(x, y), Some(Hit::Search));
        assert_eq!(m.hit(540.0, m.clock_baseline - 30.0), None);
    }

    #[test]
    fn dock_is_evenly_spaced_around_the_all_apps_button() {
        let m = m();
        let xs: Vec<f32> = [0, 1, 2, 3, 4].iter().map(|&i| if i == 2 { m.all_apps_rect().0 + m.all_apps_rect().2 / 2.0 } else { m.dock_icon_rect(i).0 + m.dock_icon_rect(i).2 / 2.0 }).collect();
        let step = xs[1] - xs[0];
        for w in xs.windows(2) {
            assert!((w[1] - w[0] - step).abs() < 0.01);
        }
        assert!((xs[2] - 540.0).abs() < 0.01);
    }
}
