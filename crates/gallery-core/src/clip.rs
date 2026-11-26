//! Clipping a video: where the two handles may go, how long the clip is,
//! and roughly how big the new file will be.

/// The shortest clip: half a second.
pub const MIN_MS: u64 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipRange {
    pub start_ms: u64,
    pub end_ms: u64,
    pub dur_ms: u64,
}

impl ClipRange {
    /// The whole video.
    pub fn new(dur_ms: u64) -> ClipRange {
        ClipRange { start_ms: 0, end_ms: dur_ms, dur_ms }
    }

    /// Move the start handle; it stays at least `MIN_MS` before the end.
    pub fn set_start(&mut self, ms: u64) {
        self.start_ms = ms.min(self.end_ms.saturating_sub(MIN_MS));
    }

    /// Move the end handle; it stays at least `MIN_MS` after the start.
    pub fn set_end(&mut self, ms: u64) {
        self.end_ms = ms.clamp((self.start_ms + MIN_MS).min(self.dur_ms), self.dur_ms);
    }

    pub fn len_ms(&self) -> u64 {
        self.end_ms - self.start_ms
    }

    pub fn is_whole(&self) -> bool {
        self.start_ms == 0 && self.end_ms == self.dur_ms
    }
}

/// About how big the clip will be: its share of the original's size, less
/// the sound's share (`audio_share`, 0..1) when the sound is left out.
pub fn estimate_bytes(file_size: u64, dur_ms: u64, len_ms: u64, keep_sound: bool, audio_share: f32) -> u64 {
    if dur_ms == 0 {
        return 0;
    }
    let part = file_size as f64 * len_ms as f64 / dur_ms as f64;
    let part = if keep_sound { part } else { part * (1.0 - audio_share.clamp(0.0, 1.0) as f64) };
    part.round() as u64
}

/// `0:12.4`, or `1:02:03.4` past an hour.
pub fn time_label(ms: u64) -> String {
    let tenths = ms / 100;
    let (h, m, s, t) = (tenths / 36_000, tenths / 600 % 60, tenths / 10 % 60, tenths % 10);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{t}") } else { format!("{m}:{s:02}.{t}") }
}

/// Whole seconds: `1:04`, `1:02:03`.
fn whole_label(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// The middle label: `18.6 s of 1:04`, or `1:15.0 of 1:02:03` for a minute or more.
pub fn length_label(len_ms: u64, dur_ms: u64) -> String {
    if len_ms < 60_000 {
        format!("{}.{} s of {}", len_ms / 1000, len_ms / 100 % 10, whole_label(dur_ms))
    } else {
        format!("{} of {}", time_label(len_ms), whole_label(dur_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_keep_order_and_minimum() {
        let mut c = ClipRange::new(10_000);
        assert_eq!((c.start_ms, c.end_ms), (0, 10_000));
        assert!(c.is_whole());
        c.set_start(4_000);
        c.set_end(3_000);
        // The end can't come closer than the minimum.
        assert_eq!((c.start_ms, c.end_ms), (4_000, 4_500));
        c.set_start(9_000);
        assert_eq!(c.start_ms, 4_000);
        c.set_end(20_000);
        assert_eq!(c.end_ms, 10_000);
        c.set_start(9_800);
        assert_eq!(c.start_ms, 9_500);
        assert_eq!(c.len_ms(), 500);
        assert!(!c.is_whole());
        // A video shorter than the minimum keeps all of it.
        let short = ClipRange::new(300);
        assert_eq!((short.start_ms, short.end_ms), (0, 300));
    }

    #[test]
    fn estimate_scales_with_length() {
        // Half of a 48 MB video, with sound: about 24 MB.
        assert_eq!(estimate_bytes(48_000_000, 64_000, 32_000, true, 0.03), 24_000_000);
        // Without sound the sound's share comes off.
        assert_eq!(estimate_bytes(48_000_000, 64_000, 32_000, false, 0.03), 23_280_000);
        assert_eq!(estimate_bytes(48_000_000, 0, 0, true, 0.0), 0);
    }

    #[test]
    fn labels() {
        assert_eq!(time_label(12_400), "0:12.4");
        assert_eq!(time_label(64_000), "1:04.0");
        assert_eq!(time_label(0), "0:00.0");
        assert_eq!(time_label(3_723_400), "1:02:03.4");
        assert_eq!(length_label(18_600, 64_000), "18.6 s of 1:04");
        assert_eq!(length_label(75_000, 3_723_000), "1:15.0 of 1:02:03");
    }
}
