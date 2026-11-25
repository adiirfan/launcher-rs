//! Where things sit on the Holo app list screen, in pixels.

use crate::geometry::Insets;

/// At least this many columns; more on wide screens, each at least
/// CELL_MIN_DP wide.
pub const MIN_COLS: usize = 4;
pub const CELL_MIN_DP: f32 = 100.0;

#[derive(Clone, Copy, Debug)]
pub struct DrawerMetrics {
    pub screen_w: f32,
    pub screen_h: f32,
    pub density: f32,
    pub insets: Insets,
    pub icon: f32,
    /// The tab header ("APPS"): x, y, w, h.
    pub header: (f32, f32, f32, f32),
    pub field: (f32, f32, f32, f32),
    pub list_top: f32,
    pub list_left: f32,
    pub cols: usize,
    pub cell_w: f32,
    pub row_h: f32,
    /// Space above the first row inside the list.
    pub pad_top: f32,
    pub label_gap: f32,
    /// The HOME bar along the bottom: x, y, w, h.
    pub home_bar: (f32, f32, f32, f32),
}

impl DrawerMetrics {
    pub fn new(screen_w: f32, screen_h: f32, density: f32, insets: Insets, icon: f32) -> DrawerMetrics {
        let d = density;
        let inner_w = screen_w - insets.left - insets.right;
        let header = (insets.left, insets.top, inner_w, 56.0 * d);
        let field = (insets.left + 20.0 * d, header.1 + header.3 + 14.0 * d, inner_w - 40.0 * d, 44.0 * d);
        let bar_h = 64.0 * d;
        let list_w = inner_w - 16.0 * d;
        let cols = ((list_w / (CELL_MIN_DP * d)) as usize).max(MIN_COLS);
        DrawerMetrics {
            screen_w,
            screen_h,
            density,
            insets,
            icon,
            header,
            field,
            list_top: field.1 + field.3 + 6.0 * d,
            list_left: insets.left + 8.0 * d,
            cols,
            cell_w: list_w / cols as f32,
            row_h: icon + 6.0 * d + 18.0 * d + 22.0 * d,
            pad_top: 14.0 * d,
            label_gap: 6.0 * d,
            home_bar: (0.0, screen_h - insets.bottom - bar_h, screen_w, bar_h),
        }
    }

    /// Height of the list's contents: `rows` of apps plus `extra` below them.
    pub fn content_height(&self, rows: usize, extra: f32) -> f32 {
        self.pad_top + rows as f32 * self.row_h + extra + 24.0 * self.density
    }

    /// Height of the window onto the list, down to the HOME bar.
    pub fn visible_height(&self) -> f32 {
        self.home_bar.1 - self.list_top
    }

    pub fn cell_rect(&self, i: usize, scroll: f32) -> (f32, f32, f32, f32) {
        let (col, row) = ((i % self.cols) as f32, (i / self.cols) as f32);
        (self.list_left + col * self.cell_w, self.list_top + self.pad_top + row * self.row_h - scroll, self.cell_w, self.row_h)
    }

    pub fn icon_rect(&self, i: usize, scroll: f32) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.cell_rect(i, scroll);
        (x + (w - self.icon) / 2.0, y, self.icon, self.icon)
    }

    pub fn hit_cell(&self, x: f32, y: f32, scroll: f32, count: usize) -> Option<usize> {
        if y < self.list_top || y >= self.home_bar.1 {
            return None;
        }
        let rx = x - self.list_left;
        if rx < 0.0 || rx >= self.cell_w * self.cols as f32 {
            return None;
        }
        let ry = y - self.list_top - self.pad_top + scroll;
        if ry < 0.0 {
            return None;
        }
        let i = (ry / self.row_h) as usize * self.cols + (rx / self.cell_w) as usize;
        (i < count).then_some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> DrawerMetrics {
        DrawerMetrics::new(1080.0, 2160.0, 3.0, Insets { top: 72.0, bottom: 144.0, left: 0.0, right: 0.0 }, 150.0)
    }

    fn centre(r: (f32, f32, f32, f32)) -> (f32, f32) {
        (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
    }

    #[test]
    fn header_then_field_then_list() {
        let m = m();
        assert_eq!(m.header, (0.0, 72.0, 1080.0, 168.0));
        assert_eq!(m.field, (60.0, 282.0, 960.0, 132.0));
        assert_eq!(m.list_top, 432.0);
    }

    #[test]
    fn home_bar_sits_on_the_nav_bar() {
        let m = m();
        assert_eq!(m.home_bar, (0.0, 1824.0, 1080.0, 192.0));
        assert_eq!(m.visible_height(), 1824.0 - 432.0);
    }

    #[test]
    fn four_cells_fill_the_width() {
        let m = m();
        assert_eq!(m.cell_rect(0, 0.0).0, 24.0);
        let last = m.cell_rect(3, 0.0);
        assert!((last.0 + last.2 - 1056.0).abs() < 0.01);
        assert_eq!(m.cell_rect(4, 0.0).1, m.cell_rect(0, 0.0).1 + m.row_h);
    }

    #[test]
    fn wide_screen_fits_more_columns() {
        let m = DrawerMetrics::new(2504.0, 2256.0, 3.0, Insets { top: 72.0, bottom: 72.0, left: 0.0, right: 0.0 }, 150.0);
        assert_eq!(m.cols, 8);
        let last = m.cell_rect(7, 0.0);
        assert!((last.0 + last.2 - (2504.0 - 24.0)).abs() < 0.01);
        assert_eq!(m.cell_rect(8, 0.0).1, m.cell_rect(0, 0.0).1 + m.row_h);
        let (x, y) = centre(m.icon_rect(13, 0.0));
        assert_eq!(m.hit_cell(x, y, 0.0, 61), Some(13));
    }

    #[test]
    fn narrow_screen_keeps_four_columns() {
        assert_eq!(m().cols, 4);
    }

    #[test]
    fn hits_follow_the_scroll_and_stop_at_the_home_bar() {
        let m = m();
        let (x, y) = centre(m.icon_rect(5, 0.0));
        assert_eq!(m.hit_cell(x, y, 0.0, 61), Some(5));
        assert_eq!(m.hit_cell(x, y, m.row_h, 61), Some(9));
        assert_eq!(m.hit_cell(x, m.list_top - 1.0, 0.0, 61), None);
        assert_eq!(m.hit_cell(x, m.home_bar.1 + 5.0, 0.0, 61), None);
        assert_eq!(m.hit_cell(x, y, 0.0, 5), None);
        assert_eq!(m.hit_cell(5.0, y, 0.0, 61), None, "left margin");
    }

    #[test]
    fn content_height_counts_rows_and_extra() {
        let m = m();
        let h = m.content_height(16, 0.0);
        assert!((m.content_height(16, 300.0) - h - 300.0).abs() < 0.01);
        assert!(h >= 16.0 * m.row_h);
    }
}
