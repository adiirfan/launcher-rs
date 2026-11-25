//! A chain of fonts: each character comes from the first font that has it.
//! Glyphs are rasterised one at a time, when first needed.

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, point};

use crate::text::GlyphMetrics;

/// A glyph's coverage (0..=255 per pixel), placed relative to the pen:
/// `left` px right of it and `top` px below the baseline (negative = above).
#[derive(Debug)]
pub struct GlyphBitmap {
    pub w: u32,
    pub h: u32,
    pub left: f32,
    pub top: f32,
    pub coverage: Vec<u8>,
}

pub struct FontSet {
    fonts: Vec<FontRef<'static>>,
}

impl FontSet {
    /// `fonts` in fallback order.
    pub fn new(fonts: Vec<FontRef<'static>>) -> FontSet {
        FontSet { fonts }
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    fn find(&self, c: char) -> Option<(&FontRef<'static>, GlyphId)> {
        self.fonts.iter().find_map(|f| {
            let id = f.glyph_id(c);
            (id.0 != 0).then_some((f, id))
        })
    }

    pub fn ascent(&self, px: f32) -> f32 {
        self.fonts.first().map_or(0.0, |f| f.as_scaled(PxScale::from(px)).ascent())
    }

    /// Android sizes text by its em; ab_glyph scales by line height
    /// (ascent - descent). Convert an em size in px to ab_glyph's scale.
    pub fn px_for_em(&self, em_px: f32) -> f32 {
        let Some(f) = self.fonts.first() else { return em_px };
        match f.units_per_em() {
            Some(upm) if upm > 0.0 => em_px * f.height_unscaled() / upm,
            _ => em_px,
        }
    }

    pub fn raster(&self, c: char, px: f32) -> Option<GlyphBitmap> {
        if c.is_whitespace() {
            return None;
        }
        let (font, id) = self.find(c)?;
        let outlined = font.outline_glyph(id.with_scale_and_position(PxScale::from(px), point(0.0, 0.0)))?;
        let b = outlined.px_bounds();
        let (w, h) = (b.width().ceil() as u32, b.height().ceil() as u32);
        if w == 0 || h == 0 {
            return None;
        }
        let mut coverage = vec![0u8; (w * h) as usize];
        outlined.draw(|x, y, v| {
            if x < w && y < h {
                coverage[(y * w + x) as usize] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        });
        Some(GlyphBitmap { w, h, left: b.min.x, top: b.min.y, coverage })
    }
}

impl GlyphMetrics for FontSet {
    fn advance(&self, c: char, px: f32) -> f32 {
        self.find(c).map_or(0.0, |(f, id)| f.as_scaled(PxScale::from(px)).h_advance(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::GlyphMetrics;

    const DEJAVU: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

    fn dejavu() -> Option<ab_glyph::FontRef<'static>> {
        let Ok(bytes) = std::fs::read(DEJAVU) else {
            eprintln!("skipping: {DEJAVU} not found");
            return None;
        };
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        Some(ab_glyph::FontRef::try_from_slice(bytes).unwrap())
    }

    #[test]
    fn advances_differ_by_letter() {
        let Some(f) = dejavu() else { return };
        let set = FontSet::new(vec![f]);
        assert!(set.advance('W', 20.0) > set.advance('i', 20.0));
        assert!(set.advance('i', 20.0) > 0.0);
    }

    #[test]
    fn rasterises_a_letter_but_not_a_space() {
        let Some(f) = dejavu() else { return };
        let set = FontSet::new(vec![f]);
        let a = set.raster('A', 32.0).expect("A");
        assert!(a.w > 0 && a.h > 0);
        assert_eq!(a.coverage.len(), (a.w * a.h) as usize);
        assert!(a.coverage.iter().any(|&c| c > 200));
        assert!(a.top < 0.0, "A sits above the baseline");
        assert!(set.raster(' ', 32.0).is_none());
        assert!(set.ascent(32.0) > 0.0);
    }

    #[test]
    fn missing_glyph_in_every_font() {
        let (Some(a), Some(b)) = (dejavu(), dejavu()) else { return };
        let set = FontSet::new(vec![a, b]);
        assert_eq!(set.advance('設', 20.0), 0.0);
        assert!(set.raster('設', 20.0).is_none());
        assert!(set.advance('A', 20.0) > 0.0);
    }

    #[test]
    fn em_size_converts_to_a_larger_line_height() {
        let Some(f) = dejavu() else { return };
        let set = FontSet::new(vec![f]);
        let px = set.px_for_em(20.0);
        assert!(px > 20.0 && px < 30.0, "{px}");
    }

    #[test]
    fn empty_set_is_harmless() {
        let set = FontSet::new(vec![]);
        assert!(set.is_empty());
        assert_eq!(set.advance('A', 20.0), 0.0);
        assert!(set.raster('A', 20.0).is_none());
        assert_eq!(set.ascent(20.0), 0.0);
        assert_eq!(set.px_for_em(20.0), 20.0);
    }
}
