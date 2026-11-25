//! Cutting icon pixels to a shape before they're saved.

/// Multiply every premultiplied pixel of a `size`×`size` RGBA image by how
/// much of it lies inside a rounded square with corner `radius` (a smooth
/// one-pixel edge).
pub fn round_square_mask(rgba: &mut [u8], size: u32, radius: f32) {
    let half = size as f32 / 2.0;
    let r = radius.min(half);
    for y in 0..size {
        for x in 0..size {
            // Signed distance from the pixel centre to the rounded square.
            let px = (x as f32 + 0.5 - half).abs() - (half - r);
            let py = (y as f32 + 0.5 - half).abs() - (half - r);
            let outside = (px.max(0.0).powi(2) + py.max(0.0).powi(2)).sqrt();
            let d = outside + px.max(py).min(0.0) - r;
            let cover = (0.5 - d).clamp(0.0, 1.0);
            if cover >= 1.0 {
                continue;
            }
            let i = ((y * size + x) * 4) as usize;
            for c in &mut rgba[i..i + 4] {
                *c = (*c as f32 * cover).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white(size: u32) -> Vec<u8> {
        vec![255; (size * size * 4) as usize]
    }

    fn px(img: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    #[test]
    fn centre_kept_corner_cut() {
        let mut img = white(100);
        round_square_mask(&mut img, 100, 18.0);
        assert_eq!(px(&img, 100, 50, 50), [255; 4]);
        assert_eq!(px(&img, 100, 0, 0)[3], 0);
        assert_eq!(px(&img, 100, 99, 99)[3], 0);
    }

    #[test]
    fn straight_edges_stay_solid() {
        let mut img = white(100);
        round_square_mask(&mut img, 100, 18.0);
        assert_eq!(px(&img, 100, 50, 1)[3], 255);
        assert_eq!(px(&img, 100, 1, 50)[3], 255);
        assert_eq!(px(&img, 100, 98, 50)[3], 255);
    }

    #[test]
    fn edge_pixels_stay_premultiplied() {
        let mut img = white(100);
        round_square_mask(&mut img, 100, 18.0);
        for y in 0..100 {
            for x in 0..100 {
                let p = px(&img, 100, x, y);
                assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3], "{x},{y} {p:?}");
            }
        }
        // Somewhere on the curved corner, a partly covered pixel.
        assert!((0..20).any(|i| { let a = px(&img, 100, i, 5)[3]; a > 0 && a < 255 }));
    }
}
