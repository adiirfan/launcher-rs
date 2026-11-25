//! Everything drawn in a frame, as one list of instances for one draw call.

use bytemuck::{Pod, Zeroable};

use crate::{Rect, Rgba};

pub const KIND_SHAPE: f32 = 0.0;
pub const KIND_IMAGE: f32 = 1.0;
pub const KIND_GLYPH: f32 = 2.0;

/// One shape, image or glyph. The shader reads exactly this layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Instance {
    /// x, y, w, h in pixels.
    pub rect: [f32; 4],
    /// Premultiplied colour (for images: opacity in every channel).
    pub color: [f32; 4],
    /// u0, v0, u1, v1 in the atlas, 0..=1.
    pub uv: [f32; 4],
    /// Corner radius, border width (0 = filled), softness (0 = crisp), kind.
    pub params: [f32; 4],
    /// Image mask radius (0 = square); the rest is reserved.
    pub extra: [f32; 4],
}

#[derive(Default)]
pub struct Batch {
    items: Vec<Instance>,
}

impl Batch {
    pub fn new() -> Batch {
        Batch::default()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn instances(&self) -> &[Instance] {
        &self.items
    }

    fn push(&mut self, r: Rect, color: [f32; 4], uv: [f32; 4], params: [f32; 4], extra: [f32; 4]) {
        self.items.push(Instance { rect: [r.x, r.y, r.w, r.h], color, uv, params, extra });
    }

    pub fn rect(&mut self, r: Rect, color: Rgba, radius: f32) {
        self.push(r, color.premul(), [0.0; 4], [radius, 0.0, 0.0, KIND_SHAPE], [0.0; 4]);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Rgba) {
        let r = Rect::new(cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        self.rect(r, color, radius);
    }

    pub fn outline(&mut self, r: Rect, color: Rgba, radius: f32, width: f32) {
        self.push(r, color.premul(), [0.0; 4], [radius, width, 0.0, KIND_SHAPE], [0.0; 4]);
    }

    /// Axis-aligned lines only.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Rgba) {
        debug_assert!(x0 == x1 || y0 == y1, "only horizontal or vertical lines");
        let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
        let r = if dx >= dy {
            Rect::new(x0.min(x1), y0 - width / 2.0, dx, width)
        } else {
            Rect::new(x0 - width / 2.0, y0.min(y1), width, dy)
        };
        self.rect(r, color, 0.0);
    }

    /// A soft halo `spread` px wide around `r`.
    pub fn glow(&mut self, r: Rect, color: Rgba, radius: f32, spread: f32) {
        let g = Rect::new(r.x - spread, r.y - spread, r.w + spread * 2.0, r.h + spread * 2.0);
        self.push(g, color.premul(), [0.0; 4], [radius + spread, 0.0, spread, KIND_SHAPE], [0.0; 4]);
    }

    /// An icon from the icon atlas, optionally masked to a rounded shape.
    pub fn image(&mut self, r: Rect, uv: [f32; 4], mask_radius: f32, opacity: f32) {
        self.push(r, [opacity; 4], uv, [0.0, 0.0, 0.0, KIND_IMAGE], [mask_radius, 0.0, 0.0, 0.0]);
    }

    /// A glyph from the glyph atlas, tinted with `color`.
    pub fn glyph(&mut self, r: Rect, uv: [f32; 4], color: Rgba) {
        self.push(r, color.premul(), uv, [0.0, 0.0, 0.0, KIND_GLYPH], [0.0; 4]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Rect, Rgba};

    fn only(b: &Batch) -> Instance {
        assert_eq!(b.instances().len(), 1);
        b.instances()[0]
    }

    #[test]
    fn instance_is_five_vec4s() {
        assert_eq!(std::mem::size_of::<Instance>(), 80);
    }

    #[test]
    fn rect_is_a_premultiplied_rounded_shape() {
        let mut b = Batch::new();
        b.rect(Rect::new(1.0, 2.0, 3.0, 4.0), Rgba::hex(0xFF0000).with_alpha(0.5), 6.0);
        let i = only(&b);
        assert_eq!(i.rect, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(i.color, [0.5, 0.0, 0.0, 0.5]);
        assert_eq!(i.params, [6.0, 0.0, 0.0, KIND_SHAPE]);
    }

    #[test]
    fn circle_is_a_rect_with_half_size_radius() {
        let mut b = Batch::new();
        b.circle(50.0, 60.0, 10.0, Rgba::hex(0xFFFFFF));
        let i = only(&b);
        assert_eq!(i.rect, [40.0, 50.0, 20.0, 20.0]);
        assert_eq!(i.params, [10.0, 0.0, 0.0, KIND_SHAPE]);
    }

    #[test]
    fn outline_sets_border_width() {
        let mut b = Batch::new();
        b.outline(Rect::new(0.0, 0.0, 10.0, 10.0), Rgba::hex(0xFFFFFF), 3.0, 2.0);
        assert_eq!(only(&b).params, [3.0, 2.0, 0.0, KIND_SHAPE]);
    }

    #[test]
    fn glow_grows_by_spread_and_softens() {
        let mut b = Batch::new();
        b.glow(Rect::new(10.0, 10.0, 20.0, 20.0), Rgba::hex(0x33B5E5), 4.0, 8.0);
        let i = only(&b);
        assert_eq!(i.rect, [2.0, 2.0, 36.0, 36.0]);
        assert_eq!(i.params, [12.0, 0.0, 8.0, KIND_SHAPE]);
    }

    #[test]
    fn horizontal_and_vertical_lines() {
        let mut b = Batch::new();
        b.line(0.0, 5.0, 100.0, 5.0, 2.0, Rgba::hex(0xFFFFFF));
        b.line(7.0, 0.0, 7.0, 50.0, 1.0, Rgba::hex(0xFFFFFF));
        assert_eq!(b.instances()[0].rect, [0.0, 4.0, 100.0, 2.0]);
        assert_eq!(b.instances()[1].rect, [6.5, 0.0, 1.0, 50.0]);
    }

    #[test]
    fn image_and_glyph_kinds() {
        let mut b = Batch::new();
        b.image(Rect::new(0.0, 0.0, 48.0, 48.0), [0.0, 0.0, 0.5, 0.5], 24.0, 0.8);
        b.glyph(Rect::new(1.0, 1.0, 8.0, 12.0), [0.1, 0.1, 0.2, 0.2], Rgba::hex(0xFFFFFF));
        let img = b.instances()[0];
        assert_eq!(img.params[3], KIND_IMAGE);
        assert_eq!(img.uv, [0.0, 0.0, 0.5, 0.5]);
        assert_eq!(img.extra[0], 24.0);
        assert_eq!(img.color, [0.8; 4]);
        let gl = b.instances()[1];
        assert_eq!(gl.params[3], KIND_GLYPH);
        assert_eq!(gl.color, [1.0; 4]);
    }

    #[test]
    fn clear_empties() {
        let mut b = Batch::new();
        b.circle(0.0, 0.0, 1.0, Rgba::hex(0));
        b.clear();
        assert!(b.instances().is_empty());
    }
}
