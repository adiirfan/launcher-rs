//! Where every heading and tile sits in the scrolling grid.

use std::ops::Range;

use crate::arrange::Arranged;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpec {
    pub width_px: f32,
    pub density: f32,
    pub cols: u32,
    pub gap_px: f32,
    /// A group heading's full height, margins included.
    pub header_px: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row {
    Header { group: u32, y: f32 },
    /// `first` indexes `Arranged::order`.
    Tiles { group: u32, first: u32, count: u32, y: f32 },
}

impl Row {
    pub fn y(&self) -> f32 {
        match *self {
            Row::Header { y, .. } | Row::Tiles { y, .. } => y,
        }
    }
    pub fn group(&self) -> u32 {
        match *self {
            Row::Header { group, .. } | Row::Tiles { group, .. } => group,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grid {
    pub rows: Vec<Row>,
    pub tile_px: f32,
    pub height: f32,
}

/// Columns when the owner hasn't pinched: 4 on a phone, more on wide
/// screens (the unfolded Fold), about one per 110 dp.
pub fn default_cols(width_dp: f32) -> u32 {
    if width_dp < 600.0 { 4 } else { ((width_dp / 110.0).round() as u32).max(4) }
}

/// One column more or fewer per pinch that grows past 1.25× or shrinks
/// under 0.8×; from 3 up to 7 (more on wide screens).
pub fn pinch_cols(current: u32, scale: f32, width_dp: f32) -> u32 {
    let max = 7.max((width_dp / 80.0) as u32);
    let next = if scale > 1.25 {
        current.saturating_sub(1)
    } else if scale < 0.8 {
        current + 1
    } else {
        current
    };
    next.clamp(3, max.max(current))
}

pub fn lay_out(a: &Arranged, spec: &GridSpec) -> Grid {
    let cols = spec.cols.max(1);
    let tile = ((spec.width_px - spec.gap_px * (cols - 1) as f32) / cols as f32).max(1.0);
    let step = tile + spec.gap_px;
    let mut rows = Vec::with_capacity(a.groups.len() * 2 + a.order.len() / cols as usize);
    let mut y = 0.0;
    for (gi, g) in a.groups.iter().enumerate() {
        rows.push(Row::Header { group: gi as u32, y });
        y += spec.header_px;
        let mut first = g.start;
        let end = g.start + g.len;
        while first < end {
            let count = cols.min(end - first);
            rows.push(Row::Tiles { group: gi as u32, first, count, y });
            y += step;
            first += count;
        }
    }
    Grid { rows, tile_px: tile, height: y }
}

impl Grid {
    fn row_height(&self, i: usize) -> f32 {
        let next = self.rows.get(i + 1).map_or(self.height, |r| r.y());
        next - self.rows[i].y()
    }

    /// The row holding content height `y` (the last row starting at or above it).
    fn row_at(&self, y: f32) -> Option<usize> {
        let i = self.rows.partition_point(|r| r.y() <= y);
        i.checked_sub(1)
    }

    /// Rows that show any part of `scroll_y .. scroll_y + view_h`.
    pub fn visible(&self, scroll_y: f32, view_h: f32) -> Range<usize> {
        let start = self.row_at(scroll_y).unwrap_or(0);
        let start = if start < self.rows.len() && self.rows[start].y() + self.row_height(start) <= scroll_y {
            start + 1
        } else {
            start
        };
        let end = self.rows.partition_point(|r| r.y() < scroll_y + view_h);
        start.min(self.rows.len())..end.max(start.min(self.rows.len()))
    }

    /// The item (index into `Arranged::order`) under content point `x, y`.
    pub fn item_at(&self, x: f32, y: f32, spec: &GridSpec) -> Option<u32> {
        if y < 0.0 || y >= self.height || x < 0.0 {
            return None;
        }
        let Row::Tiles { first, count, y: top, .. } = self.rows[self.row_at(y)?] else { return None };
        if y - top >= self.tile_px {
            return None;
        }
        let step = self.tile_px + spec.gap_px;
        let col = (x / step) as u32;
        if col >= count || x - col as f32 * step >= self.tile_px {
            return None;
        }
        Some(first + col)
    }

    /// The first item of the tile row at the top of the view: the one to
    /// keep at the top when the columns change.
    pub fn anchor(&self, scroll_y: f32) -> Option<u32> {
        self.rows.iter().find_map(|r| match *r {
            Row::Tiles { first, y, .. } if y + self.tile_px > scroll_y => Some(first),
            _ => None,
        })
    }

    /// The top of the tile row holding `item`.
    pub fn scroll_for(&self, item: u32) -> f32 {
        let i = self.rows.partition_point(|r| match *r {
            Row::Tiles { first, count, .. } => first + count <= item,
            Row::Header { .. } => true,
        });
        // Headers sit before their rows, so skip back over none: the found
        // index is a tile row, or the end.
        match self.rows.get(i) {
            Some(Row::Tiles { y, .. }) => *y,
            _ => self.rows.last().map_or(0.0, |r| r.y()),
        }
    }

    /// The heading of whatever is at the top of the view, for the
    /// fast-scroll bubble.
    pub fn label_at<'a>(&self, a: &'a Arranged, scroll_y: f32) -> Option<&'a str> {
        let i = self.row_at(scroll_y.max(0.0)).unwrap_or(0);
        let row = self.rows.get(i)?;
        a.groups.get(row.group() as usize).map(|g| g.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::Group;

    fn arranged(sizes: &[u32]) -> Arranged {
        let mut groups = Vec::new();
        let mut start = 0;
        for (i, &n) in sizes.iter().enumerate() {
            groups.push(Group { label: format!("G{i}"), start, len: n });
            start += n;
        }
        Arranged { order: (0..start).collect(), groups }
    }

    fn spec(cols: u32) -> GridSpec {
        let d = 2.625;
        GridSpec { width_px: 1081.0, density: d, cols, gap_px: 2.0 * d, header_px: 48.0 * d }
    }

    #[test]
    fn four_cols_on_a_412dp_phone() {
        assert_eq!(default_cols(412.0), 4);
        assert_eq!(default_cols(700.0), 6);
        assert_eq!(default_cols(300.0), 4);
        let g = lay_out(&arranged(&[4]), &spec(4));
        let want = (1081.0 - 3.0 * 2.0 * 2.625) / 4.0;
        assert!((g.tile_px - want).abs() < 0.01, "{}", g.tile_px);
    }

    #[test]
    fn headers_then_rows() {
        let s = spec(4);
        let g = lay_out(&arranged(&[5, 3]), &s);
        let kinds: Vec<(char, u32, u32)> = g
            .rows
            .iter()
            .map(|r| match *r {
                Row::Header { group, .. } => ('H', group, 0),
                Row::Tiles { first, count, .. } => ('T', first, count),
            })
            .collect();
        assert_eq!(kinds, [('H', 0, 0), ('T', 0, 4), ('T', 4, 1), ('H', 1, 0), ('T', 5, 3)]);
        let step = g.tile_px + s.gap_px;
        assert_eq!(g.rows[1].y(), s.header_px);
        assert_eq!(g.rows[2].y(), s.header_px + step);
        assert!((g.height - (2.0 * s.header_px + 3.0 * step)).abs() < 0.01);
    }

    #[test]
    fn item_at_hits_and_misses() {
        let s = spec(4);
        let g = lay_out(&arranged(&[5, 3]), &s);
        let step = g.tile_px + s.gap_px;
        let y1 = s.header_px + 5.0;
        assert_eq!(g.item_at(5.0, y1, &s), Some(0));
        assert_eq!(g.item_at(step * 3.0 + 5.0, y1, &s), Some(3));
        assert_eq!(g.item_at(5.0, s.header_px + step + 5.0, &s), Some(4));
        // Second tile row has one item: the empty cells are not items.
        assert_eq!(g.item_at(step + 5.0, s.header_px + step + 5.0, &s), None);
        // In the gap between two tiles.
        assert_eq!(g.item_at(g.tile_px + 1.0, y1, &s), None);
        // On a heading.
        assert_eq!(g.item_at(5.0, 5.0, &s), None);
        assert_eq!(g.item_at(5.0, -5.0, &s), None);
        assert_eq!(g.item_at(5.0, g.height + 5.0, &s), None);
    }

    #[test]
    fn visible_rows() {
        let s = spec(4);
        let g = lay_out(&arranged(&[40]), &s);
        assert_eq!(g.visible(0.0, s.header_px - 1.0), 0..1);
        let r = g.visible(s.header_px + 0.5, g.tile_px);
        assert_eq!(r.start, 1);
        assert!(r.end == 2 || r.end == 3);
        assert_eq!(g.visible(g.height + 10.0, 100.0), g.rows.len()..g.rows.len());
    }

    #[test]
    fn pinch_steps_one_at_a_time() {
        assert_eq!(pinch_cols(4, 1.3, 412.0), 3);
        assert_eq!(pinch_cols(4, 0.7, 412.0), 5);
        assert_eq!(pinch_cols(4, 1.1, 412.0), 4);
        assert_eq!(pinch_cols(3, 2.0, 412.0), 3);
        assert_eq!(pinch_cols(7, 0.5, 412.0), 7);
        assert_eq!(pinch_cols(7, 0.5, 860.0), 8);
        assert_eq!(pinch_cols(10, 0.5, 860.0), 10);
    }

    #[test]
    fn anchor_kept_across_column_change() {
        let g4 = lay_out(&arranged(&[100]), &spec(4));
        let scroll = g4.scroll_for(37);
        let anchor = g4.anchor(scroll).unwrap();
        assert!(anchor <= 37 && 37 < anchor + 4);
        let g6 = lay_out(&arranged(&[100]), &spec(6));
        let y = g6.scroll_for(anchor);
        let row = g6.rows.iter().find(|r| r.y() == y).unwrap();
        let Row::Tiles { first, count, .. } = *row else { panic!("not a tile row") };
        assert!(first <= 37 && 37 < first + count);
    }

    #[test]
    fn label_follows_scroll() {
        let s = spec(4);
        let a = arranged(&[8, 8]);
        let g = lay_out(&a, &s);
        assert_eq!(g.label_at(&a, 0.0), Some("G0"));
        let second = g.rows.iter().find(|r| matches!(r, Row::Header { group: 1, .. })).unwrap().y();
        assert_eq!(g.label_at(&a, second - 1.0), Some("G0"));
        assert_eq!(g.label_at(&a, second), Some("G1"));
        assert_eq!(g.label_at(&a, g.height + 100.0), Some("G1"));
        assert_eq!(lay_out(&arranged(&[]), &s).label_at(&arranged(&[]), 0.0), None);
    }
}
