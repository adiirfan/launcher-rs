//! Making a GIF from part of a video: which moments become frames, in
//! which order they play, how big the GIF comes out, and roughly how many
//! bytes that is.

/// Over this, the screen warns that some apps refuse the file.
pub const TOO_BIG: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plays {
    Loop,
    /// Forwards then backwards.
    Bounce,
    Backwards,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GifPlan {
    pub start_ms: u64,
    pub end_ms: u64,
    pub fps: u32,
    /// Width in pixels; 0 keeps the video's own.
    pub width: u32,
    pub plays: Plays,
}

/// Bytes a GIF pixel takes, roughly, for video frames after compression.
const BYTES_PER_PIXEL: f64 = 0.45;

impl GifPlan {
    /// When each frame is taken from the video, in ms.
    pub fn frame_times(&self) -> Vec<u64> {
        let fps = self.fps.max(1) as u64;
        let n = (self.end_ms.saturating_sub(self.start_ms) * fps / 1000).max(1);
        (0..n).map(|i| self.start_ms + i * 1000 / fps).collect()
    }

    /// Which of `n` frames play, in order.
    pub fn order(&self, n: usize) -> Vec<usize> {
        match self.plays {
            Plays::Loop => (0..n).collect(),
            Plays::Backwards => (0..n).rev().collect(),
            Plays::Bounce => (0..n).chain((1..n.saturating_sub(1)).rev()).collect(),
        }
    }

    /// The GIF's size for a video of `w` x `h` (upright); never bigger than the video.
    pub fn size(&self, w: u32, h: u32) -> (u32, u32) {
        if self.width == 0 || self.width >= w || w == 0 {
            return (w, h);
        }
        (self.width, ((h as u64 * self.width as u64 + w as u64 / 2) / w as u64).max(1) as u32)
    }

    pub fn estimate_bytes(&self, w: u32, h: u32) -> u64 {
        let (gw, gh) = self.size(w, h);
        let frames = self.order(self.frame_times().len()).len();
        (frames as f64 * gw as f64 * gh as f64 * BYTES_PER_PIXEL) as u64
    }

    /// How long frame `i` shows, in hundredths of a second (GIF's unit),
    /// spread so the total never drifts from real time.
    pub fn delay_cs(&self, i: usize) -> u16 {
        let fps = self.fps.max(1) as u64;
        let at = |k: u64| (k * 100 + fps / 2) / fps;
        (at(i as u64 + 1) - at(i as u64)).max(1) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(start: u64, end: u64, fps: u32, width: u32, plays: Plays) -> GifPlan {
        GifPlan { start_ms: start, end_ms: end, fps, width, plays }
    }

    #[test]
    fn frame_times_cover_range() {
        let p = plan(1000, 2000, 10, 480, Plays::Loop);
        let t = p.frame_times();
        assert_eq!(t.len(), 10);
        assert_eq!((t[0], t[1], t[9]), (1000, 1100, 1900));
        // 15 a second: thirds of a tenth, rounded down.
        let t = plan(0, 1000, 15, 480, Plays::Loop).frame_times();
        assert_eq!((t.len(), t[1], t[14]), (15, 66, 933));
        // Always at least one frame.
        assert_eq!(plan(500, 500, 15, 480, Plays::Loop).frame_times(), vec![500]);
    }

    #[test]
    fn bounce_order() {
        assert_eq!(plan(0, 1, 1, 1, Plays::Bounce).order(4), vec![0, 1, 2, 3, 2, 1]);
        assert_eq!(plan(0, 1, 1, 1, Plays::Bounce).order(1), vec![0]);
        assert_eq!(plan(0, 1, 1, 1, Plays::Loop).order(3), vec![0, 1, 2]);
    }

    #[test]
    fn backwards_order() {
        assert_eq!(plan(0, 1, 1, 1, Plays::Backwards).order(3), vec![2, 1, 0]);
    }

    #[test]
    fn size_keeps_ratio_and_never_grows() {
        assert_eq!(plan(0, 1, 1, 480, Plays::Loop).size(1920, 1080), (480, 270));
        assert_eq!(plan(0, 1, 1, 640, Plays::Loop).size(320, 240), (320, 240));
        assert_eq!(plan(0, 1, 1, 0, Plays::Loop).size(1080, 1920), (1080, 1920));
        assert_eq!(plan(0, 1, 1, 480, Plays::Loop).size(1080, 1920), (480, 853));
    }

    #[test]
    fn estimate_grows_with_width_and_fps() {
        let base = plan(0, 6000, 15, 480, Plays::Loop);
        let e = base.estimate_bytes(1920, 1080);
        assert!(e > 3_000_000 && e < 8_000_000, "{e}");
        assert!(GifPlan { fps: 30, ..base }.estimate_bytes(1920, 1080) > e);
        assert!(GifPlan { width: 640, ..base }.estimate_bytes(1920, 1080) > e);
        // Bounce plays nearly twice the frames.
        assert!(GifPlan { plays: Plays::Bounce, ..base }.estimate_bytes(1920, 1080) > e * 3 / 2);
    }

    #[test]
    fn delays_add_up() {
        // 15 a second: 7, 6, 7, 7, 6, 7 … hundredths, never drifting.
        let d: Vec<u16> = (0..15).map(|i| plan(0, 1, 15, 1, Plays::Loop).delay_cs(i)).collect();
        assert_eq!(d.iter().map(|&x| x as u32).sum::<u32>(), 100);
        assert!(d.iter().all(|&x| x == 6 || x == 7));
    }
}
