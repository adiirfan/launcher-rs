//! The launcher's drawing kit: everything that can be tested without a phone.

pub mod anim;
pub mod atlas;
pub mod batch;
pub mod font;
pub mod text;

/// A rectangle in pixels, y down.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Left and top edges are inside, right and bottom edges are not.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// A straight (not premultiplied) colour, components 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub [f32; 4]);

impl Rgba {
    pub fn hex(rgb: u32) -> Rgba {
        let c = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
        Rgba([c(16), c(8), c(0), 1.0])
    }

    pub fn with_alpha(self, a: f32) -> Rgba {
        let [r, g, b, _] = self.0;
        Rgba([r, g, b, a])
    }

    /// What the GPU blends with: colour multiplied by alpha.
    pub fn premul(self) -> [f32; 4] {
        let [r, g, b, a] = self.0;
        [r * a, g * a, b * a, a]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_hex_and_premul() {
        assert_eq!(
            Rgba::hex(0x33B5E5).0,
            [0x33 as f32 / 255.0, 0xB5 as f32 / 255.0, 0xE5 as f32 / 255.0, 1.0]
        );
        assert_eq!(Rgba::hex(0xFFFFFF).with_alpha(0.5).premul(), [0.5, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect::new(10.0, 10.0, 20.0, 20.0);
        assert!(r.contains(10.0, 10.0));
        assert!(r.contains(29.9, 29.9));
        assert!(!r.contains(30.0, 30.0));
        assert!(!r.contains(9.9, 10.0));
    }
}
pub mod slots;
pub mod shape;
