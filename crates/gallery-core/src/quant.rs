//! Turning a frame's millions of colours into a GIF's 256, and finding the
//! part of a frame that changed since the last one.

use color_quant::NeuQuant;

/// A frame's own 256-colour palette (RGB, 768 bytes) and each pixel's place in it.
pub fn reduce(rgba: &[u8], w: usize, h: usize) -> (Vec<u8>, Vec<u8>) {
    let px = &rgba[..w * h * 4];
    let nq = NeuQuant::new(10, 256, px);
    let palette = nq.color_map_rgb();
    let idx = px.chunks_exact(4).map(|p| nq.index_of(p) as u8).collect();
    (palette, idx)
}

/// The box (x, y, w, h) around every pixel that differs between two
/// frames of the same size; None when they're the same.
pub fn changed_rect(prev: &[u8], cur: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        let row = y * w * 4..(y + 1) * w * 4;
        let (a, b) = (&prev[row.clone()], &cur[row]);
        if a == b {
            continue;
        }
        let first = (0..w).find(|&x| a[x * 4..x * 4 + 4] != b[x * 4..x * 4 + 4]).unwrap_or(0);
        let last = (0..w).rev().find(|&x| a[x * 4..x * 4 + 4] != b[x * 4..x * 4 + 4]).unwrap_or(w - 1);
        x0 = x0.min(first);
        x1 = x1.max(last);
        y0 = y0.min(y);
        y1 = y;
    }
    (y0 != usize::MAX).then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_keeps_flat_colour() {
        let (w, h) = (16, 8);
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [200u8, 40, 90, 255]).collect();
        let (palette, idx) = reduce(&rgba, w, h);
        assert_eq!(idx.len(), w * h);
        let i = idx[0] as usize;
        assert!(idx.iter().all(|&x| x as usize == i), "a flat colour stays one index");
        let c = &palette[i * 3..i * 3 + 3];
        assert!(c.iter().zip([200u8, 40, 90]).all(|(a, b)| (*a as i32 - b as i32).abs() <= 4), "{c:?}");
    }

    #[test]
    fn changed_rect_finds_box() {
        let (w, h) = (10, 6);
        let a = vec![0u8; w * h * 4];
        let mut b = a.clone();
        for (x, y) in [(3, 1), (6, 4)] {
            b[(y * w + x) * 4] = 255;
        }
        assert_eq!(changed_rect(&a, &b, w, h), Some((3, 1, 4, 4)));
        assert_eq!(changed_rect(&a, &a, w, h), None);
    }
}
