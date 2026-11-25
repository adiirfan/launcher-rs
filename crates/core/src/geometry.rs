//! Where everything sits on the home screen, in pixels.

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
    Pill,
}

pub const DOCK_SLOTS: u32 = 5;

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
    /// Gap between an icon's bottom edge and its label's top.
    pub label_gap: f32,
    pub dock_left: f32,
    pub dock_slot_w: f32,
    pub dock_top: f32,
    pub dock_h: f32,
    /// Centre line of the page dots.
    pub dots_y: f32,
    /// Search pill: x, y, w, h.
    pub pill: (f32, f32, f32, f32),
    pub date_left: f32,
    pub date_baseline: f32,
}

impl HomeMetrics {
    pub fn new(screen_w: f32, screen_h: f32, density: f32, insets: Insets, cols: u32, rows: u32) -> HomeMetrics {
        let d = density;
        let inner_w = screen_w - insets.left - insets.right;

        let pill_h = 52.0 * d;
        let pill_bottom = screen_h - insets.bottom - 16.0 * d;
        let pill = (insets.left + 20.0 * d, pill_bottom - pill_h, inner_w - 40.0 * d, pill_h);

        let dock_h = 72.0 * d;
        let dock_top = pill.1 - 8.0 * d - dock_h;
        let dots_y = dock_top - 8.0 * d;
        let grid_bottom = dock_top - 16.0 * d;

        let grid_top = insets.top + 96.0 * d;
        let grid_left = insets.left + 8.0 * d;
        let cell_w = (inner_w - 16.0 * d) / cols as f32;
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
            grid_left,
            grid_top,
            label_gap: 6.0 * d,
            dock_left: insets.left + 12.0 * d,
            dock_slot_w: (inner_w - 24.0 * d) / DOCK_SLOTS as f32,
            dock_top,
            dock_h,
            dots_y,
            pill,
            date_left: insets.left + 24.0 * d,
            date_baseline: insets.top + 56.0 * d,
        }
    }

    pub fn cell_rect(&self, col: u32, row: u32) -> (f32, f32, f32, f32) {
        (
            self.grid_left + col as f32 * self.cell_w,
            self.grid_top + row as f32 * self.cell_h,
            self.cell_w,
            self.cell_h,
        )
    }

    pub fn icon_rect(&self, col: u32, row: u32) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.cell_rect(col, row);
        let group = self.icon + self.label_gap + 18.0 * self.density;
        (x + (w - self.icon) / 2.0, y + (h - group) / 2.0, self.icon, self.icon)
    }

    pub fn dock_icon_rect(&self, i: u32) -> (f32, f32, f32, f32) {
        let x = self.dock_left + i as f32 * self.dock_slot_w + (self.dock_slot_w - self.icon) / 2.0;
        let y = self.dock_top + (self.dock_h - self.icon) / 2.0;
        (x, y, self.icon, self.icon)
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let inside = |r: (f32, f32, f32, f32)| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3;
        if inside(self.pill) {
            return Some(Hit::Pill);
        }
        let dock = (self.dock_left, self.dock_top, self.dock_slot_w * DOCK_SLOTS as f32, self.dock_h);
        if inside(dock) {
            let i = ((x - self.dock_left) / self.dock_slot_w) as u32;
            return Some(Hit::Dock(i.min(DOCK_SLOTS - 1)));
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
        HomeMetrics::new(1080.0, 2160.0, 3.0, ins, 4, 5)
    }

    fn centre(r: (f32, f32, f32, f32)) -> (f32, f32) {
        (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
    }

    #[test]
    fn icon_is_56dp_when_there_is_room() {
        let ins = Insets { top: 72.0, bottom: 144.0, left: 0.0, right: 0.0 };
        let tall = HomeMetrics::new(1080.0, 2400.0, 2.625, ins, 4, 5);
        assert_eq!(tall.icon, 56.0 * 2.625);
    }

    #[test]
    fn icon_and_label_fit_in_a_short_cell() {
        let m = m();
        assert!(m.icon <= 168.0);
        assert!(m.icon >= 40.0 * 3.0, "icon too small: {}", m.icon);
        assert!(m.icon + m.label_gap + 18.0 * 3.0 <= m.cell_h + 0.01);
    }

    #[test]
    fn everything_stays_inside_the_bars() {
        let m = m();
        assert!(m.grid_top >= 72.0 + 288.0 - 0.01);
        assert!(m.date_baseline > 72.0);
        assert!(m.grid_top + m.rows as f32 * m.cell_h <= m.dock_top + 0.01);
        assert!(m.dock_top + m.dock_h <= m.pill.1 + 0.01);
        assert!(m.pill.1 + m.pill.3 <= 2160.0 - 144.0 - 48.0 + 0.01);
        assert_eq!(m.pill.3, 52.0 * 3.0);
    }

    #[test]
    fn side_insets_push_the_grid_in() {
        let ins = Insets { top: 72.0, bottom: 144.0, left: 100.0, right: 60.0 };
        let m = HomeMetrics::new(1080.0, 2160.0, 3.0, ins, 4, 5);
        assert!(m.grid_left >= 100.0);
        assert!(m.grid_left + 4.0 * m.cell_w <= 1080.0 - 60.0 + 0.01);
        assert!(m.pill.0 >= 100.0 && m.pill.0 + m.pill.2 <= 1020.0 + 0.01);
    }

    #[test]
    fn cells_tile_without_overlap() {
        let m = m();
        let a = m.cell_rect(0, 0);
        let b = m.cell_rect(1, 0);
        let c = m.cell_rect(0, 1);
        assert!((a.0 + a.2 - b.0).abs() < 0.01);
        assert!((a.1 + a.3 - c.1).abs() < 0.01);
        let last = m.cell_rect(3, 4);
        assert!((last.0 + last.2 - (1080.0 - m.grid_left)).abs() < 0.01);
    }

    #[test]
    fn icon_sits_inside_its_cell() {
        let m = m();
        let c = m.cell_rect(2, 3);
        let i = m.icon_rect(2, 3);
        assert!(i.0 >= c.0 && i.0 + i.2 <= c.0 + c.2);
        assert!(i.1 >= c.1 && i.1 + i.3 <= c.1 + c.3);
        assert_eq!(i.2, m.icon);
    }

    #[test]
    fn hits() {
        let m = m();
        let (x, y) = centre(m.icon_rect(2, 3));
        assert_eq!(m.hit(x, y), Some(Hit::Cell { col: 2, row: 3 }));
        let (x, y) = centre(m.dock_icon_rect(4));
        assert_eq!(m.hit(x, y), Some(Hit::Dock(4)));
        let (x, y) = centre(m.pill);
        assert_eq!(m.hit(x, y), Some(Hit::Pill));
        assert_eq!(m.hit(540.0, m.date_baseline), None);
        assert_eq!(m.hit(540.0, 10.0), None);
    }

    #[test]
    fn dock_is_evenly_spaced() {
        let m = m();
        let xs: Vec<f32> = (0..5).map(|i| m.dock_icon_rect(i).0).collect();
        let step = xs[1] - xs[0];
        assert!(step > 0.0);
        for w in xs.windows(2) {
            assert!((w[1] - w[0] - step).abs() < 0.01);
        }
    }
}
