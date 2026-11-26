//! Zooming and panning one picture in the viewer. Screen pixels per image
//! pixel is the scale; the picture's top-left corner sits at `tx, ty`.

/// One image pixel may grow to this many screen pixels, no more.
pub const MAX_SCALE: f32 = 4.0;
/// Double tap zooms to this many times the fitted size.
pub const DOUBLE_TAP: f32 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    image: (f32, f32),
    view: (f32, f32),
    fit_scale: f32,
    scale: f32,
    tx: f32,
    ty: f32,
}

impl Zoom {
    /// The whole picture on screen, centred.
    pub fn fit(image: (f32, f32), view: (f32, f32)) -> Zoom {
        let fit_scale = (view.0 / image.0.max(1.0)).min(view.1 / image.1.max(1.0));
        let mut z = Zoom { image, view, fit_scale, scale: fit_scale, tx: 0.0, ty: 0.0 };
        z.clamp();
        z
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    fn max_scale(&self) -> f32 {
        MAX_SCALE.max(self.fit_scale)
    }

    /// Whether it's showing the whole picture.
    pub fn is_fit(&self) -> bool {
        self.scale <= self.fit_scale * 1.001
    }

    /// Centre an axis that fits; keep the edges on screen for one that doesn't.
    fn clamp(&mut self) {
        let (w, h) = (self.image.0 * self.scale, self.image.1 * self.scale);
        self.tx = if w <= self.view.0 { (self.view.0 - w) / 2.0 } else { self.tx.clamp(self.view.0 - w, 0.0) };
        self.ty = if h <= self.view.1 { (self.view.1 - h) / 2.0 } else { self.ty.clamp(self.view.1 - h, 0.0) };
    }

    fn zoom_to(&mut self, scale: f32, focus: (f32, f32)) {
        let new = scale.clamp(self.fit_scale, self.max_scale());
        let k = new / self.scale;
        self.tx = focus.0 - (focus.0 - self.tx) * k;
        self.ty = focus.1 - (focus.1 - self.ty) * k;
        self.scale = new;
        self.clamp();
    }

    /// Two fingers moved apart by `scale`, around screen point `focus`.
    pub fn pinch(&mut self, scale: f32, focus: (f32, f32)) {
        self.zoom_to(self.scale * scale, focus);
    }

    /// Drag by `dx, dy`. Returns the part of `dx` the picture couldn't use
    /// because its edge was reached, so the pager can move instead.
    pub fn pan(&mut self, dx: f32, dy: f32) -> f32 {
        let before = self.tx;
        self.tx += dx;
        self.ty += dy;
        self.clamp();
        dx - (self.tx - before)
    }

    /// Fitted → 2.5× around the tap; zoomed → fitted.
    pub fn double_tap(&mut self, at: (f32, f32)) {
        if self.is_fit() {
            self.zoom_to(self.fit_scale * DOUBLE_TAP, at);
        } else {
            *self = Zoom::fit(self.image, self.view);
        }
    }

    /// Where the picture is drawn on screen: x, y, w, h.
    pub fn dest(&self) -> (f32, f32, f32, f32) {
        (self.tx, self.ty, self.image.0 * self.scale, self.image.1 * self.scale)
    }

    /// The part of the picture on screen, in image pixels: x, y, w, h.
    pub fn source_rect(&self) -> (f32, f32, f32, f32) {
        let x0 = (-self.tx / self.scale).max(0.0);
        let y0 = (-self.ty / self.scale).max(0.0);
        let x1 = ((self.view.0 - self.tx) / self.scale).min(self.image.0);
        let y1 = ((self.view.1 - self.ty) / self.scale).min(self.image.1);
        (x0, y0, x1 - x0, y1 - y0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01 && (a.2 - b.2).abs() < 0.01 && (a.3 - b.3).abs() < 0.01
    }

    #[test]
    fn fit_centres_landscape() {
        let z = Zoom::fit((4000.0, 3000.0), (1000.0, 2000.0));
        assert_eq!(z.scale(), 0.25);
        assert!(z.is_fit());
        assert!(close(z.dest(), (0.0, 625.0, 1000.0, 750.0)), "{:?}", z.dest());
        assert!(close(z.source_rect(), (0.0, 0.0, 4000.0, 3000.0)), "{:?}", z.source_rect());
    }

    #[test]
    fn double_tap_round_trip() {
        let mut z = Zoom::fit((4000.0, 3000.0), (1000.0, 2000.0));
        let fitted = z;
        z.double_tap((500.0, 1000.0));
        assert!((z.scale() - 0.625).abs() < 1e-4);
        assert!(!z.is_fit());
        // The tapped point stays put.
        let (x, y, w, h) = z.dest();
        assert!((x + w / 2.0 - 500.0).abs() < 0.01 && (y + h / 2.0 - 1000.0).abs() < 0.01, "{:?}", z.dest());
        z.double_tap((100.0, 100.0));
        assert_eq!(z, fitted);
    }

    #[test]
    fn pan_returns_leftover_at_edge() {
        let mut z = Zoom::fit((4000.0, 3000.0), (1000.0, 2000.0));
        // Fitted: the picture can't move sideways at all.
        assert_eq!(z.pan(100.0, 0.0), 100.0);
        z.double_tap((500.0, 1000.0));
        assert!((z.dest().0 + 750.0).abs() < 0.01);
        assert_eq!(z.pan(300.0, 0.0), 0.0);
        assert!((z.pan(600.0, 0.0) - 150.0).abs() < 0.01);
        assert_eq!(z.dest().0, 0.0);
        // Leftover keeps its direction.
        let mut z2 = Zoom::fit((4000.0, 3000.0), (1000.0, 2000.0));
        z2.double_tap((500.0, 1000.0));
        assert!((z2.pan(-1000.0, 0.0) + 250.0).abs() < 0.01);
    }

    #[test]
    fn source_rect_at_2x() {
        let mut z = Zoom::fit((1000.0, 1000.0), (1000.0, 1000.0));
        z.pinch(2.0, (500.0, 500.0));
        assert_eq!(z.scale(), 2.0);
        assert!(close(z.source_rect(), (250.0, 250.0, 500.0, 500.0)), "{:?}", z.source_rect());
    }

    #[test]
    fn zoom_limits() {
        let mut z = Zoom::fit((1000.0, 1000.0), (1000.0, 1000.0));
        z.pinch(100.0, (500.0, 500.0));
        assert_eq!(z.scale(), MAX_SCALE);
        z.pinch(0.001, (500.0, 500.0));
        assert!(z.is_fit());
        // A tiny picture is fitted larger than 4×: it can't zoom further, nor shrink.
        let mut t = Zoom::fit((50.0, 50.0), (1000.0, 1000.0));
        assert_eq!(t.scale(), 20.0);
        t.pinch(2.0, (500.0, 500.0));
        assert_eq!(t.scale(), 20.0);
    }
}
