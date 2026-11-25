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

/// Some apps ship an icon picture with a wide empty border, so it looks
/// small next to the others. For a `size`×`size` RGBA drawing of one, the
/// bounds (left, top, right, bottom) to draw it at again so the artwork
/// fills the square, centred; None if it already nearly does.
pub fn fill_bounds(rgba: &[u8], size: u32) -> Option<(i32, i32, i32, i32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (size, size, 0, 0);
    for y in 0..size {
        for x in 0..size {
            if rgba[((y * size + x) * 4 + 3) as usize] > 16 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    if x1 <= x0 {
        return None;
    }
    let side = (x1 - x0).max(y1 - y0) as f32;
    let n = size as f32;
    if side / n >= 0.85 {
        return None;
    }
    let scale = (n / side).min(2.5);
    let (cx, cy) = ((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
    let (left, top) = (n / 2.0 - cx * scale, n / 2.0 - cy * scale);
    Some((left.round() as i32, top.round() as i32, (left + n * scale).round() as i32, (top + n * scale).round() as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white(size: u32) -> Vec<u8> {
        vec![255; (size * size * 4) as usize]
    }

    /// A `size` image, clear except an opaque square from `a` to `b`
    /// (exclusive) on both axes.
    fn square(size: u32, a: u32, b: u32) -> Vec<u8> {
        let mut img = vec![0; (size * size * 4) as usize];
        for y in a..b {
            for x in a..b {
                let i = ((y * size + x) * 4) as usize;
                img[i..i + 4].copy_from_slice(&[200, 0, 0, 255]);
            }
        }
        img
    }

    #[test]
    fn padded_icon_is_drawn_bigger_to_fill_the_square() {
        // Artwork in the middle half of a 100 px frame: draw it twice as big,
        // centred on the artwork.
        let img = square(100, 25, 75);
        assert_eq!(fill_bounds(&img, 100), Some((-50, -50, 150, 150)));
        // Off-centre artwork is moved to the middle too.
        let img = square(100, 10, 60);
        assert_eq!(fill_bounds(&img, 100), Some((-20, -20, 180, 180)));
    }

    #[test]
    fn nearly_full_or_empty_icons_are_left_alone() {
        assert_eq!(fill_bounds(&square(100, 5, 95), 100), None);
        assert_eq!(fill_bounds(&square(100, 0, 0), 100), None);
        // A tiny dot is not blown up past 2.5x.
        assert_eq!(fill_bounds(&square(100, 45, 55), 100), Some((-75, -75, 175, 175)));
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
