//! Where things sit on the app list screen, in pixels.

use crate::geometry::Insets;

pub const COLS: usize = 4;

#[derive(Clone, Copy, Debug)]
pub struct DrawerMetrics {
    pub screen_w: f32,
    pub screen_h: f32,
    pub density: f32,
    pub insets: Insets,
    pub icon: f32,
    pub field: (f32, f32, f32, f32),
    pub list_top: f32,
    pub list_left: f32,
    pub cell_w: f32,
    pub row_h: f32,
    /// Space above the first row inside the list.
    pub pad_top: f32,
    pub label_gap: f32,
    pub back_button: (f32, f32, f32, f32),
}

impl DrawerMetrics {
    pub fn new(screen_w: f32, screen_h: f32, density: f32, insets: Insets, icon: f32) -> DrawerMetrics {
        let d = density;
        let inner_w = screen_w - insets.left - insets.right;
        let field = (insets.left + 20.0 * d, insets.top + 20.0 * d, inner_w - 40.0 * d, 52.0 * d);
        let back_w = 150.0 * d;
        let back_h = 44.0 * d;
        DrawerMetrics {
            screen_w,
            screen_h,
            density,
            insets,
            icon,
            field,
            list_top: field.1 + field.3 + 16.0 * d,
            list_left: insets.left + 12.0 * d,
            cell_w: (inner_w - 24.0 * d) / COLS as f32,
            row_h: icon + 8.0 * d + 18.0 * d + 22.0 * d,
            pad_top: 4.0 * d,
            label_gap: 8.0 * d,
            back_button: (
                screen_w - insets.right - 20.0 * d - back_w,
                screen_h - insets.bottom - 24.0 * d - back_h,
                back_w,
                back_h,
            ),
        }
    }

    /// Height of the list's contents: `rows` of apps plus `extra` below them.
    /// The end leaves room so the last row clears the "Back to Home" button.
    pub fn content_height(&self, rows: usize, extra: f32) -> f32 {
        self.pad_top + rows as f32 * self.row_h + extra + 32.0 * self.density + self.back_button.3
    }

    /// Height of the window onto the list.
    pub fn visible_height(&self) -> f32 {
        self.screen_h - self.list_top - self.insets.bottom
    }

    pub fn cell_rect(&self, i: usize, scroll: f32) -> (f32, f32, f32, f32) {
        let (col, row) = ((i % COLS) as f32, (i / COLS) as f32);
        (self.list_left + col * self.cell_w, self.list_top + self.pad_top + row * self.row_h - scroll, self.cell_w, self.row_h)
    }

    pub fn icon_rect(&self, i: usize, scroll: f32) -> (f32, f32, f32, f32) {
        let (x, y, w, _) = self.cell_rect(i, scroll);
        (x + (w - self.icon) / 2.0, y, self.icon, self.icon)
    }

    pub fn hit_cell(&self, x: f32, y: f32, scroll: f32, count: usize) -> Option<usize> {
        if y < self.list_top || y > self.screen_h - self.insets.bottom {
            return None;
        }
        let rx = x - self.list_left;
        if rx < 0.0 || rx >= self.cell_w * COLS as f32 {
            return None;
        }
        let ry = y - self.list_top - self.pad_top + scroll;
        if ry < 0.0 {
            return None;
        }
        let i = (ry / self.row_h) as usize * COLS + (rx / self.cell_w) as usize;
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
    fn field_sits_below_the_status_bar() {
        let m = m();
        assert_eq!(m.field, (60.0, 132.0, 960.0, 156.0));
        assert!(m.list_top >= m.field.1 + m.field.3);
    }

    #[test]
    fn four_cells_fill_the_width() {
        let m = m();
        let first = m.cell_rect(0, 0.0);
        let last = m.cell_rect(3, 0.0);
        assert_eq!(first.0, 36.0);
        assert!((last.0 + last.2 - (1080.0 - 36.0)).abs() < 0.01);
        assert_eq!(m.cell_rect(4, 0.0).1, first.1 + m.row_h);
    }

    #[test]
    fn hits_follow_the_scroll() {
        let m = m();
        let (x, y) = centre(m.icon_rect(5, 0.0));
        assert_eq!(m.hit_cell(x, y, 0.0, 61), Some(5));
        assert_eq!(m.hit_cell(x, y, m.row_h, 61), Some(9));
        assert_eq!(m.hit_cell(x, m.list_top - 1.0, 0.0, 61), None);
        assert_eq!(m.hit_cell(x, y, 0.0, 5), None);
        assert_eq!(m.hit_cell(5.0, y, 0.0, 61), None, "left margin");
    }

    #[test]
    fn content_height_counts_rows_and_extra() {
        let m = m();
        let h = m.content_height(16, 0.0);
        assert!((m.content_height(16, 300.0) - h - 300.0).abs() < 0.01);
        assert!(h >= 16.0 * m.row_h);
        assert!(m.visible_height() < 2160.0 - m.list_top);
    }

    #[test]
    fn back_button_above_the_nav_bar() {
        let m = m();
        let (x, y, w, h) = m.back_button;
        assert!(y + h <= 2160.0 - 144.0 - 72.0 + 0.01);
        assert!(x + w <= 1080.0 - 60.0 + 0.01);
        assert_eq!(h, 132.0);
    }
}
