//! Saving a GIF as a video: how long each GIF frame shows, how many video
//! frames that becomes at a steady rate, and how much data a second.

/// How the video is squeezed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Good,
    Smaller,
}

/// How long a GIF frame shows, from its delay in hundredths of a second.
/// Browsers show 0 and 1 as a tenth of a second, so the video does too.
pub fn delay_ms(cs: u16) -> u64 {
    if cs <= 1 { 100 } else { cs as u64 * 10 }
}

/// The video's steady frame step, and how many video frames each GIF frame
/// fills. Every frame ends where the GIF's would, to the nearest step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timing {
    pub step_ms: u64,
    pub counts: Vec<u32>,
}

pub fn timing(delays_ms: &[u64]) -> Timing {
    let step_ms = delays_ms.iter().copied().min().unwrap_or(100).max(20);
    let mut end = 0u64;
    let mut shown = 0u64;
    let counts = delays_ms
        .iter()
        .map(|&d| {
            end += d;
            let upto = (end + step_ms / 2) / step_ms;
            let n = upto.saturating_sub(shown);
            shown += n;
            n as u32
        })
        .collect();
    Timing { step_ms, counts }
}

/// Bits a second for a `w` x `h` video at `fps`.
pub fn bit_rate(w: u32, h: u32, fps: u32, q: Quality) -> u32 {
    let px = w as f64 * h as f64 * fps.max(1) as f64;
    let (per_px, lo, hi) = match q {
        Quality::Good => (0.25, 500_000.0, 20_000_000.0),
        Quality::Smaller => (0.07, 200_000.0, 6_000_000.0),
    };
    (px * per_px).clamp(lo, hi) as u32
}

/// The video's size for a `w` x `h` GIF made `k` times bigger: even sides.
pub fn video_size(w: u32, h: u32, k: u32) -> (u32, u32) {
    crate::yuv::even_size(w * k.max(1), h * k.max(1))
}

/// A `w` x `h` RGBA picture made `k` times bigger (each pixel a k x k block),
/// cut to `out_w` x `out_h` from the top left, see-through parts on white.
pub fn video_frame(rgba: &[u8], w: usize, h: usize, k: usize, out_w: usize, out_h: usize) -> Vec<u8> {
    let k = k.max(1);
    let mut out = vec![255u8; out_w * out_h * 4];
    for y in 0..out_h.min(h * k) {
        let src_row = &rgba[(y / k) * w * 4..];
        for x in 0..out_w.min(w * k) {
            let s = &src_row[(x / k) * 4..(x / k) * 4 + 4];
            let a = s[3] as u32;
            let at = (y * out_w + x) * 4;
            for c in 0..3 {
                out[at + c] = ((s[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_delays_show_for_a_tenth() {
        assert_eq!(delay_ms(0), 100);
        assert_eq!(delay_ms(1), 100);
        assert_eq!(delay_ms(2), 20);
        assert_eq!(delay_ms(7), 70);
    }

    #[test]
    fn even_delays_make_one_frame_each() {
        let t = timing(&[40, 40, 40]);
        assert_eq!(t.step_ms, 40);
        assert_eq!(t.counts, vec![1, 1, 1]);
    }

    #[test]
    fn a_long_frame_is_held() {
        // A 2 s still between quick frames.
        let t = timing(&[50, 2000, 50]);
        assert_eq!(t.step_ms, 50);
        assert_eq!(t.counts, vec![1, 40, 1]);
    }

    #[test]
    fn uneven_delays_keep_the_total_length() {
        let d = [30, 40, 30, 40, 30];
        let t = timing(&d);
        assert_eq!(t.step_ms, 30);
        let total: u64 = t.counts.iter().map(|&c| c as u64).sum::<u64>() * t.step_ms;
        let want: u64 = d.iter().sum();
        assert!(total.abs_diff(want) <= t.step_ms / 2, "{total} vs {want}");
    }

    #[test]
    fn no_step_shorter_than_50_a_second() {
        assert_eq!(timing(&[10, 10]).step_ms, 20);
    }

    #[test]
    fn smaller_uses_less_data() {
        let good = bit_rate(480, 270, 25, Quality::Good);
        let small = bit_rate(480, 270, 25, Quality::Smaller);
        assert!(small * 3 <= good, "{small} vs {good}");
        assert!(bit_rate(100, 100, 10, Quality::Smaller) >= 200_000);
        assert!(bit_rate(4000, 4000, 50, Quality::Good) <= 20_000_000);
    }

    #[test]
    fn sizes_are_even_and_scaled() {
        assert_eq!(video_size(481, 271, 1), (480, 270));
        assert_eq!(video_size(33, 21, 3), (98, 62));
    }

    #[test]
    fn frames_grow_in_blocks_on_white() {
        // 2 x 1: red, then see-through.
        let rgba = [255, 0, 0, 255, 0, 0, 0, 0];
        let out = video_frame(&rgba, 2, 1, 2, 4, 2);
        assert_eq!(out.len(), 4 * 2 * 4);
        let px = |x: usize, y: usize| &out[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
        assert_eq!(px(1, 1), &[255, 0, 0, 255]);
        assert_eq!(px(2, 0), &[255, 255, 255, 255]);
        // Cut short: the odd column goes.
        let out = video_frame(&[9, 9, 9, 255, 9, 9, 9, 255, 7, 7, 7, 255, 7, 7, 7, 255, 7, 7, 7, 255, 7, 7, 7, 255], 3, 2, 1, 2, 2);
        assert_eq!(out, [9, 9, 9, 255, 9, 9, 9, 255, 7, 7, 7, 255, 7, 7, 7, 255]);
    }
}
