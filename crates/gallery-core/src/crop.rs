//! Crop and rotate: where the frame is, how the picture is turned, flipped
//! and straightened, and which source pixel each output pixel comes from.
//! The picture shown is the source turned by quarter turns, then mirrored
//! (FLIP), then straightened about its centre and zoomed just enough to
//! keep the frame covered.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Free,
    Original,
    Square,
    R3x2,
    R16x9,
}

impl Shape {
    pub const ALL: [Shape; 5] = [Shape::Free, Shape::Original, Shape::Square, Shape::R3x2, Shape::R16x9];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Free => "FREE",
            Shape::Original => "ORIGINAL",
            Shape::Square => "SQUARE",
            Shape::R3x2 => "3:2",
            Shape::R16x9 => "16:9",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    /// Quarter turns clockwise, 0-3.
    pub turns: u8,
    /// Mirrored left to right (after turning).
    pub flip: bool,
    /// Straighten, -45 to 45, clockwise positive.
    pub angle_deg: f32,
    /// x, y, w, h as parts (0-1) of the turned picture.
    pub rect: (f32, f32, f32, f32),
    pub shape: Shape,
}

impl Default for Crop {
    fn default() -> Crop {
        Crop { turns: 0, flip: false, angle_deg: 0.0, rect: (0.0, 0.0, 1.0, 1.0), shape: Shape::Free }
    }
}

/// Smallest crop side, as a part of the picture.
pub const MIN_PART: f32 = 0.05;

/// Size after `turns` quarter turns.
pub fn turned_size(w: u32, h: u32, turns: u8) -> (u32, u32) {
    if turns % 2 == 1 { (h, w) } else { (w, h) }
}

/// How much the straightened picture is enlarged so it still covers a
/// `w`×`h` frame.
pub fn max_inside(angle_deg: f32, w: f32, h: f32) -> f32 {
    let a = angle_deg.abs().to_radians();
    let (c, s) = (a.cos(), a.sin());
    ((w * c + h * s) / w).max((w * s + h * c) / h)
}

impl Crop {
    fn turned(&self, w: u32, h: u32) -> (f32, f32) {
        let (tw, th) = turned_size(w, h, self.turns);
        (tw as f32, th as f32)
    }

    /// The frame's ratio in parts (width part / height part) for a shape.
    fn part_ratio(&self, w: u32, h: u32) -> Option<f32> {
        let (tw, th) = self.turned(w, h);
        Self::target_ratio(self.shape, tw, th).map(|r| r * th / tw)
    }

    /// Choose a shape: the largest frame of that shape, centred.
    pub fn set_shape(&mut self, shape: Shape, w: u32, h: u32) {
        self.shape = shape;
        let Some(pr) = self.part_ratio(w, h) else { return };
        let (fw, fh) = if pr <= 1.0 { (pr, 1.0) } else { (1.0, 1.0 / pr) };
        self.rect = ((1.0 - fw) / 2.0, (1.0 - fh) / 2.0, fw, fh);
    }

    /// A quarter turn clockwise; the frame turns with the picture.
    pub fn rotate(&mut self, w: u32, h: u32) {
        let (x, y, fw, fh) = self.rect;
        self.turns = (self.turns + 1) % 4;
        self.rect = (1.0 - y - fh, x, fh, fw);
        if self.shape != Shape::Free {
            self.set_shape(self.shape, w, h);
        }
    }

    /// Mirror left to right; the frame mirrors too.
    pub fn toggle_flip(&mut self) {
        self.flip = !self.flip;
        let (x, y, fw, fh) = self.rect;
        self.rect = (1.0 - x - fw, y, fw, fh);
    }

    /// Move one corner (0 top left, 1 top right, 2 bottom left, 3 bottom
    /// right) by `dx, dy` parts; the opposite corner stays put.
    pub fn drag_corner(&mut self, corner: u8, dx: f32, dy: f32, w: u32, h: u32) {
        let (x, y, fw, fh) = self.rect;
        let (left, top) = (corner % 2 == 0, corner < 2);
        // The fixed corner.
        let ax = if left { x + fw } else { x };
        let ay = if top { y + fh } else { y };
        let mut nw = if left { fw - dx } else { fw + dx };
        let mut nh = if top { fh - dy } else { fh + dy };
        // Room up to the picture's edge from the fixed corner.
        let room_w = if left { ax } else { 1.0 - ax };
        let room_h = if top { ay } else { 1.0 - ay };
        nw = nw.clamp(MIN_PART, room_w.max(MIN_PART));
        nh = nh.clamp(MIN_PART, room_h.max(MIN_PART));
        if let Some(pr) = self.part_ratio(w, h) {
            // Follow the wider move, then fit both inside the room.
            nh = nw / pr;
            if nh > room_h {
                nh = room_h;
                nw = nh * pr;
            }
            if nw < MIN_PART || nh < MIN_PART {
                let k = (MIN_PART / nw).max(MIN_PART / nh);
                nw *= k;
                nh *= k;
            }
        }
        let nx = if left { ax - nw } else { ax };
        let ny = if top { ay - nh } else { ay };
        self.rect = (nx, ny, nw, nh);
    }

    /// Slide the frame without changing its size.
    pub fn move_by(&mut self, dx: f32, dy: f32) {
        let (x, y, fw, fh) = self.rect;
        self.rect = ((x + dx).clamp(0.0, 1.0 - fw), (y + dy).clamp(0.0, 1.0 - fh), fw, fh);
    }

    /// Only turned or flipped: a JPEG can be saved without re-encoding.
    pub fn is_rotate_only(&self) -> bool {
        let (x, y, fw, fh) = self.rect;
        self.angle_deg == 0.0 && x.abs() < 1e-4 && y.abs() < 1e-4 && (fw - 1.0).abs() < 1e-4 && (fh - 1.0).abs() < 1e-4
    }

    /// The saved picture's size, from a `w`×`h` upright source.
    pub fn output_size(&self, w: u32, h: u32) -> (u32, u32) {
        let (tw, th) = self.turned(w, h);
        (((self.rect.2 * tw).round() as u32).max(1), ((self.rect.3 * th).round() as u32).max(1))
    }

    /// The source pixel position (from a `w`×`h` upright source) that
    /// output pixel `ox, oy` shows.
    pub fn source_point(&self, ox: f32, oy: f32, w: u32, h: u32) -> (f32, f32) {
        let (tw, th) = self.turned(w, h);
        // Where on the shown (turned, flipped, straightened) picture.
        let (px, py) = (self.rect.0 * tw + ox, self.rect.1 * th + oy);
        let (cx, cy) = (tw / 2.0, th / 2.0);
        let (vx, vy) = (px - cx, py - cy);
        let k = max_inside(self.angle_deg, tw, th);
        let a = self.angle_deg.to_radians();
        let (c, s) = (a.cos(), a.sin());
        // Undo the straightening: turn back by the angle and shrink.
        let (ux, uy) = ((vx * c + vy * s) / k, (-vx * s + vy * c) / k);
        let (mut qx, qy) = (cx + ux, cy + uy);
        if self.flip {
            qx = tw - qx;
        }
        let (w, h) = (w as f32, h as f32);
        match self.turns % 4 {
            0 => (qx, qy),
            1 => (qy, h - qx),
            2 => (w - qx, h - qy),
            _ => (w - qy, qx),
        }
    }

    /// The turned picture's ratio, for `w`×`h` upright source.
    fn target_ratio(shape: Shape, tw: f32, th: f32) -> Option<f32> {
        let landscape = tw >= th;
        let r = match shape {
            Shape::Free => return None,
            Shape::Original => return Some(tw / th),
            Shape::Square => 1.0,
            Shape::R3x2 => 1.5,
            Shape::R16x9 => 16.0 / 9.0,
        };
        Some(if landscape { r } else { 1.0 / r })
    }
}

/// The EXIF orientation value for a JPEG whose pixels are stored turned
/// `rot_deg` (what the library says) and which the owner then turned by
/// `turns` quarter turns and maybe flipped.
pub fn exif_orientation(rot_deg: u16, turns: u8, flip: bool) -> u16 {
    let quarter = ((rot_deg as u32 / 90) + turns as u32) % 4;
    match (quarter, flip) {
        (0, false) => 1,
        (1, false) => 6,
        (2, false) => 3,
        (3, false) => 8,
        (0, true) => 2,
        (1, true) => 5,
        (2, true) => 4,
        _ => 7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_orientation_combines_turns_and_flip() {
        assert_eq!(exif_orientation(0, 0, false), 1);
        assert_eq!(exif_orientation(0, 1, false), 6);
        assert_eq!(exif_orientation(90, 1, false), 3);
        assert_eq!(exif_orientation(270, 1, false), 1);
        assert_eq!(exif_orientation(0, 3, false), 8);
        assert_eq!(exif_orientation(0, 0, true), 2);
        assert_eq!(exif_orientation(90, 0, true), 5);
        assert_eq!(exif_orientation(0, 2, true), 4);
        assert_eq!(exif_orientation(180, 1, true), 7);
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn quarter_turns_swap_sides() {
        assert_eq!(turned_size(4000, 3000, 0), (4000, 3000));
        assert_eq!(turned_size(4000, 3000, 1), (3000, 4000));
        assert_eq!(turned_size(4000, 3000, 2), (4000, 3000));
        assert_eq!(turned_size(4000, 3000, 3), (3000, 4000));
    }

    #[test]
    fn square_shape_is_square() {
        let mut c = Crop::default();
        c.set_shape(Shape::Square, 4000, 3000);
        let (out_w, out_h) = c.output_size(4000, 3000);
        assert_eq!((out_w, out_h), (3000, 3000));
        assert!(close((c.rect.0, c.rect.1), (0.125, 0.0)), "centred: {:?}", c.rect);
        // 3:2 on a portrait picture is 2:3.
        c.set_shape(Shape::R3x2, 3000, 4000);
        let (w, h) = c.output_size(3000, 4000);
        assert!((w as f32 / h as f32 - 2.0 / 3.0).abs() < 0.01, "{w}x{h}");
        c.set_shape(Shape::Free, 3000, 4000);
        assert_eq!(c.output_size(3000, 4000), (w, h), "Free keeps the frame");
    }

    #[test]
    fn flip_mirrors_x() {
        let c = Crop { flip: true, ..Crop::default() };
        assert!(close(c.source_point(0.0, 0.0, 100, 50), (100.0, 0.0)));
        assert!(close(c.source_point(100.0, 50.0, 100, 50), (0.0, 50.0)));
    }

    #[test]
    fn source_point_maps_corners() {
        // One clockwise turn: the output's top left is the source's bottom left.
        let c = Crop { turns: 1, ..Crop::default() };
        assert_eq!(c.output_size(100, 50), (50, 100));
        assert!(close(c.source_point(0.0, 0.0, 100, 50), (0.0, 50.0)));
        assert!(close(c.source_point(50.0, 0.0, 100, 50), (0.0, 0.0)));
        assert!(close(c.source_point(50.0, 100.0, 100, 50), (100.0, 0.0)));
        let c2 = Crop { turns: 2, ..Crop::default() };
        assert!(close(c2.source_point(0.0, 0.0, 100, 50), (100.0, 50.0)));
        let c3 = Crop { turns: 3, ..Crop::default() };
        assert!(close(c3.source_point(0.0, 0.0, 100, 50), (100.0, 0.0)));
        // A crop of the right half.
        let half = Crop { rect: (0.5, 0.0, 0.5, 1.0), ..Crop::default() };
        assert_eq!(half.output_size(100, 50), (50, 50));
        assert!(close(half.source_point(0.0, 0.0, 100, 50), (50.0, 0.0)));
    }

    #[test]
    fn straighten_zooms_to_cover() {
        assert!((max_inside(0.0, 400.0, 300.0) - 1.0).abs() < 1e-4);
        let k = max_inside(10.0, 400.0, 300.0);
        assert!(k > 1.0);
        // Every corner of the frame lands inside the source.
        let c = Crop { angle_deg: 10.0, ..Crop::default() };
        for (x, y) in [(0.0, 0.0), (400.0, 0.0), (0.0, 300.0), (400.0, 300.0)] {
            let (sx, sy) = c.source_point(x, y, 400, 300);
            assert!((-0.5..=400.5).contains(&sx) && (-0.5..=300.5).contains(&sy), "{x},{y} -> {sx},{sy}");
        }
        // The centre stays the centre.
        assert!(close(c.source_point(200.0, 150.0, 400, 300), (200.0, 150.0)));
    }

    #[test]
    fn drag_keeps_ratio_and_minimum() {
        let mut c = Crop::default();
        c.drag_corner(3, -0.5, -0.25, 400, 300);
        assert!(close((c.rect.2, c.rect.3), (0.5, 0.75)));
        c.drag_corner(3, -2.0, -2.0, 400, 300);
        assert!(c.rect.2 >= MIN_PART - 1e-4 && c.rect.3 >= MIN_PART - 1e-4, "{:?}", c.rect);
        let mut s = Crop::default();
        s.set_shape(Shape::Square, 400, 300);
        s.drag_corner(0, 0.1, 0.0, 400, 300);
        let (w, h) = s.output_size(400, 300);
        assert!((w as i32 - h as i32).abs() <= 1, "{w}x{h}");
        // Can't leave the picture.
        let mut m = Crop { rect: (0.2, 0.2, 0.5, 0.5), ..Crop::default() };
        m.move_by(0.9, -0.9);
        assert!(close((m.rect.0, m.rect.1), (0.5, 0.0)));
    }

    #[test]
    fn rotate_only_detected() {
        assert!(Crop::default().is_rotate_only());
        assert!(Crop { turns: 1, flip: true, ..Crop::default() }.is_rotate_only());
        assert!(!Crop { angle_deg: 2.0, ..Crop::default() }.is_rotate_only());
        assert!(!Crop { rect: (0.1, 0.0, 0.9, 1.0), ..Crop::default() }.is_rotate_only());
    }
}
