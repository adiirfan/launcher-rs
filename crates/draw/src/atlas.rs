//! Packs small images (icons, glyphs) into one big texture, shelf by shelf.

/// Gap left between neighbours so smooth scaling never bleeds one into another.
const PAD: u32 = 1;

#[derive(Debug)]
struct Shelf {
    y: u32,
    h: u32,
    next_x: u32,
}

#[derive(Debug)]
pub struct ShelfPacker {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_y: u32,
}

impl ShelfPacker {
    pub fn new(width: u32, height: u32) -> ShelfPacker {
        ShelfPacker { width, height, shelves: Vec::new(), next_y: 0 }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn reset(&mut self) {
        self.shelves.clear();
        self.next_y = 0;
    }

    /// Top-left corner of a free `w`×`h` slot, or `None` if it doesn't fit.
    pub fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w == 0 || h == 0 || w > self.width || h > self.height {
            return None;
        }
        for shelf in &mut self.shelves {
            if h <= shelf.h && shelf.next_x + w <= self.width {
                let at = (shelf.next_x, shelf.y);
                shelf.next_x += w + PAD;
                return Some(at);
            }
        }
        if self.next_y + h > self.height {
            return None;
        }
        let y = self.next_y;
        self.next_y += h + PAD;
        self.shelves.push(Shelf { y, h, next_x: w + PAD });
        Some((0, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_a_shelf_left_to_right_then_opens_a_new_one() {
        let mut p = ShelfPacker::new(64, 64);
        assert_eq!(p.alloc(10, 10), Some((0, 0)));
        assert_eq!(p.alloc(10, 10), Some((11, 0)));
        assert_eq!(p.alloc(10, 20), Some((0, 11)));
    }

    #[test]
    fn never_overlaps_and_stays_inside() {
        let mut p = ShelfPacker::new(256, 256);
        let mut seed: u32 = 12345;
        let mut next = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            1 + (seed >> 16) % 20
        };
        let mut placed: Vec<(u32, u32, u32, u32)> = Vec::new();
        for _ in 0..200 {
            let (w, h) = (next(), next());
            if let Some((x, y)) = p.alloc(w, h) {
                assert!(x + w <= 256 && y + h <= 256, "outside: {x},{y} {w}x{h}");
                for &(ox, oy, ow, oh) in &placed {
                    let apart = x + w <= ox || ox + ow <= x || y + h <= oy || oy + oh <= y;
                    assert!(apart, "overlap: {x},{y} {w}x{h} vs {ox},{oy} {ow}x{oh}");
                }
                placed.push((x, y, w, h));
            }
        }
        assert!(placed.len() > 50);
    }

    #[test]
    fn too_big_full_and_reset() {
        let mut p = ShelfPacker::new(64, 64);
        assert_eq!(p.alloc(65, 1), None);
        while p.alloc(16, 16).is_some() {}
        assert_eq!(p.alloc(16, 16), None);
        p.reset();
        assert_eq!(p.alloc(10, 10), Some((0, 0)));
        assert_eq!(p.size(), (64, 64));
    }
}
