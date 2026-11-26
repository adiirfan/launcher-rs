//! Video frames come out of the phone's decoder as YUV 4:2:0 (brightness
//! at full size, colour at half). These turn them into upright, resized
//! RGBA pictures, and turn pictures back into YUV for the video encoder.

/// One decoded frame's three planes, as the decoder laid them out.
pub struct Yuv420<'a> {
    /// The picture inside the planes (the rest is padding).
    pub w: usize,
    pub h: usize,
    pub left: usize,
    pub top: usize,
    pub y: &'a [u8],
    pub y_row: usize,
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub uv_row: usize,
    /// 1 for separate planes, 2 when U and V are interleaved.
    pub uv_pix: usize,
}

impl Yuv420<'_> {
    /// The frame turned upright (`rotation` degrees clockwise, as the video
    /// says to show it) and resized to `out_w` x `out_h`, as RGBA.
    pub fn to_rgba(&self, rotation: u32, out_w: usize, out_h: usize) -> Vec<u8> {
        let (w, h) = (self.w, self.h);
        let sideways = rotation == 90 || rotation == 270;
        let (uw, uh) = if sideways { (h, w) } else { (w, h) };
        let (sx, sy) = (uw as f32 / out_w as f32, uh as f32 / out_h as f32);
        // HD video is BT.709, smaller video BT.601.
        let hd = w.max(h) >= 1280 || w.min(h) >= 720;
        let (rv, gu, gv, bu) = if hd { (1.793, 0.213, 0.533, 2.112) } else { (1.596, 0.392, 0.813, 2.017) };
        let mut out = vec![0u8; out_w * out_h * 4];
        let (wf, hf) = ((w - 1) as f32, (h - 1) as f32);
        for oy in 0..out_h {
            let uy = (oy as f32 + 0.5) * sy - 0.5;
            for ox in 0..out_w {
                let ux = (ox as f32 + 0.5) * sx - 0.5;
                let (cx, cy) = match rotation {
                    90 => (uy, hf - ux),
                    180 => (wf - ux, hf - uy),
                    270 => (wf - uy, ux),
                    _ => (ux, uy),
                };
                let (cx, cy) = (cx.clamp(0.0, wf), cy.clamp(0.0, hf));
                let luma = self.luma(cx, cy);
                let (ci, cj) = ((cx as usize + self.left) / 2, (cy as usize + self.top) / 2);
                let at = cj * self.uv_row + ci * self.uv_pix;
                let u = *self.u.get(at).unwrap_or(&128) as f32 - 128.0;
                let v = *self.v.get(at).unwrap_or(&128) as f32 - 128.0;
                let yy = 1.164 * (luma - 16.0);
                let px = &mut out[(oy * out_w + ox) * 4..(oy * out_w + ox) * 4 + 4];
                px[0] = (yy + rv * v).round().clamp(0.0, 255.0) as u8;
                px[1] = (yy - gu * u - gv * v).round().clamp(0.0, 255.0) as u8;
                px[2] = (yy + bu * u).round().clamp(0.0, 255.0) as u8;
                px[3] = 255;
            }
        }
        out
    }

    /// Brightness at a point inside the picture, smoothly between samples.
    fn luma(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let p = |xx: usize, yy: usize| *self.y.get((yy + self.top) * self.y_row + xx + self.left).unwrap_or(&16) as f32;
        let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * fx;
        let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * fx;
        top + (bot - top) * fy
    }
}

/// A picture as planar YUV 4:2:0 (I420: all Y, then U, then V), BT.601
/// limited range. `w` and `h` must be even.
pub fn rgba_to_i420(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 3 / 2];
    let (ys, rest) = out.split_at_mut(w * h);
    let (us, vs) = rest.split_at_mut(w * h / 4);
    let px = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        (rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32)
    };
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = px(x, y);
            ys[y * w + x] = (16.0 + 0.257 * r + 0.504 * g + 0.098 * b).round().clamp(0.0, 255.0) as u8;
        }
    }
    for cy in 0..h / 2 {
        for cx in 0..w / 2 {
            let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px(cx * 2 + dx, cy * 2 + dy);
                r += p.0 / 4.0;
                g += p.1 / 4.0;
                b += p.2 / 4.0;
            }
            us[cy * (w / 2) + cx] = (128.0 - 0.148 * r - 0.291 * g + 0.439 * b).round().clamp(0.0, 255.0) as u8;
            vs[cy * (w / 2) + cx] = (128.0 + 0.439 * r - 0.368 * g - 0.071 * b).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Video encoders want even sides: round down, at least 2.
pub fn even_size(w: u32, h: u32) -> (u32, u32) {
    ((w & !1).max(2), (h & !1).max(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, y: u8, u: u8, v: u8) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (vec![y; w * h], vec![u; w * h / 4], vec![v; w * h / 4])
    }

    fn frame<'a>(w: usize, h: usize, p: &'a (Vec<u8>, Vec<u8>, Vec<u8>)) -> Yuv420<'a> {
        Yuv420 { w, h, left: 0, top: 0, y: &p.0, y_row: w, u: &p.1, v: &p.2, uv_row: w / 2, uv_pix: 1 }
    }

    #[test]
    fn grey_stays_grey() {
        let p = flat(8, 4, 126, 128, 128);
        let out = frame(8, 4, &p).to_rgba(0, 4, 2);
        assert_eq!(out.len(), 4 * 2 * 4);
        for px in out.chunks(4) {
            assert!(px[..3].iter().all(|&c| (c as i32 - 128).abs() <= 3), "{px:?}");
            assert_eq!(px[3], 255);
        }
    }

    #[test]
    fn turned_frames_come_out_upright() {
        // 4 x 2 frame, the top-left brightness sample marked.
        let mut p = flat(4, 2, 16, 128, 128);
        p.0[0] = 235;
        // Shown turned 90 degrees clockwise: 2 wide, 4 tall, mark top-right.
        let out = frame(4, 2, &p).to_rgba(90, 2, 4);
        let bright = |x: usize, y: usize| out[(y * 2 + x) * 4] > 200;
        assert!(bright(1, 0));
        assert!(!bright(0, 0) && !bright(0, 3) && !bright(1, 3));
        let out = frame(4, 2, &p).to_rgba(270, 2, 4);
        let bright = |x: usize, y: usize| out[(y * 2 + x) * 4] > 200;
        assert!(bright(0, 3));
    }

    #[test]
    fn rgba_to_yuv_grey_is_neutral() {
        let rgba: Vec<u8> = (0..4 * 4).flat_map(|_| [128u8, 128, 128, 255]).collect();
        let yuv = rgba_to_i420(&rgba, 4, 4);
        assert_eq!(yuv.len(), 16 + 4 + 4);
        assert!(yuv[..16].iter().all(|&y| (y as i32 - 126).abs() <= 2), "{:?}", &yuv[..16]);
        assert!(yuv[16..].iter().all(|&c| (c as i32 - 128).abs() <= 1), "{:?}", &yuv[16..]);
    }

    #[test]
    fn even_size_rounds_down() {
        assert_eq!(even_size(481, 271), (480, 270));
        assert_eq!(even_size(1, 1), (2, 2));
        assert_eq!(even_size(640, 360), (640, 360));
    }
}
