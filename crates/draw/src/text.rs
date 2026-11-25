//! Single-line text layout: place characters by advance, cut with "…".
//! No complex shaping: each character sits at the sum of the advances before it.

pub const ELLIPSIS: char = '…';

pub trait GlyphMetrics {
    fn advance(&self, c: char, px: f32) -> f32;
}

#[derive(Debug, Default, PartialEq)]
pub struct LineLayout {
    /// Each character and its x from the start of the line.
    pub glyphs: Vec<(char, f32)>,
    pub width: f32,
    pub truncated: bool,
}

pub fn layout_line(text: &str, px: f32, max_width: f32, m: &impl GlyphMetrics) -> LineLayout {
    let advances: Vec<(char, f32)> = text.chars().map(|c| (c, m.advance(c, px))).collect();
    let total: f32 = advances.iter().map(|a| a.1).sum();
    if total <= max_width {
        let mut x = 0.0;
        let glyphs = advances
            .iter()
            .map(|&(c, a)| {
                let at = (c, x);
                x += a;
                at
            })
            .collect();
        return LineLayout { glyphs, width: total, truncated: false };
    }
    let ellipsis = m.advance(ELLIPSIS, px);
    if ellipsis > max_width {
        return LineLayout { glyphs: Vec::new(), width: 0.0, truncated: true };
    }
    let mut glyphs = Vec::new();
    let mut x = 0.0;
    for &(c, a) in &advances {
        if x + a + ellipsis > max_width {
            break;
        }
        glyphs.push((c, x));
        x += a;
    }
    glyphs.push((ELLIPSIS, x));
    LineLayout { glyphs, width: x + ellipsis, truncated: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is 10 px wide, whatever the size.
    struct Fixed;
    impl GlyphMetrics for Fixed {
        fn advance(&self, _c: char, _px: f32) -> f32 {
            10.0
        }
    }

    fn chars(l: &LineLayout) -> String {
        l.glyphs.iter().map(|g| g.0).collect()
    }

    #[test]
    fn fits_whole() {
        let l = layout_line("hello", 12.0, 100.0, &Fixed);
        assert_eq!(chars(&l), "hello");
        assert_eq!(l.glyphs.iter().map(|g| g.1).collect::<Vec<_>>(), vec![0.0, 10.0, 20.0, 30.0, 40.0]);
        assert_eq!(l.width, 50.0);
        assert!(!l.truncated);
    }

    #[test]
    fn cuts_with_ellipsis() {
        let l = layout_line("hello world", 12.0, 60.0, &Fixed);
        assert_eq!(chars(&l), "hello…");
        assert_eq!(l.width, 60.0);
        assert!(l.truncated);
    }

    #[test]
    fn empty_text() {
        let l = layout_line("", 12.0, 60.0, &Fixed);
        assert!(l.glyphs.is_empty());
        assert_eq!(l.width, 0.0);
        assert!(!l.truncated);
    }

    #[test]
    fn not_even_the_ellipsis_fits() {
        let l = layout_line("abc", 12.0, 5.0, &Fixed);
        assert!(l.glyphs.is_empty());
        assert!(l.truncated);
    }

    #[test]
    fn japanese_cut() {
        let l = layout_line("設定アプリ", 12.0, 30.0, &Fixed);
        assert_eq!(chars(&l), "設定…");
    }
}
