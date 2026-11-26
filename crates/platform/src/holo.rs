//! The Holo (Android 4) look: colours and small drawn icons shared by screens.

use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use std::sync::atomic::{AtomicU32, Ordering};

static ACCENT: AtomicU32 = AtomicU32::new(0x33B5E5);

/// The accent colour the owner picked (Holo blue unless changed).
pub fn accent() -> u32 {
    ACCENT.load(Ordering::Relaxed)
}

pub fn set_accent(rgb: u32) {
    ACCENT.store(rgb, Ordering::Relaxed);
}
pub const WHITE: u32 = 0xFFFFFF;
pub const BLACK: u32 = 0x000000;
/// Search bar text and icon on Home.
pub const HINT: u32 = 0xC8C8C8;
/// Placeholder text in the app list's search field.
pub const PLACEHOLDER: u32 = 0x8A8A8A;
pub const DATE: u32 = 0xE0E0E0;
pub const QUIET: u32 = 0xBDBDBD;
pub const DIVIDER: u32 = 0x2A2A2A;
pub const ROW_LINE: u32 = 0x262626;
/// Secondary text under a row title.
pub const TEXT_QUIET: u32 = 0x9E9E9E;
/// Action bar background.
pub const BAR: u32 = 0x141414;

/// A magnifying glass centred on (cx, cy), about 22 dp across.
pub fn magnifier(batch: &mut Batch, cx: f32, cy: f32, d: f32, color: Rgba) {
    let lens = 6.5 * d;
    let (lx, ly) = (cx - 1.5 * d, cy - 1.5 * d);
    batch.outline(Rect::new(lx - lens, ly - lens, lens * 2.0, lens * 2.0), color, lens, 1.8 * d);
    // The handle, as a short run of dots along the diagonal.
    for step in 0..4 {
        let o = lens * 0.75 + (1.0 + step as f32 * 1.3) * d;
        batch.circle(lx + o, ly + o, 1.1 * d, color);
    }
}

/// A line at any angle, drawn as a tight run of dots `width` across.
pub fn stroke(batch: &mut Batch, (x0, y0): (f32, f32), (x1, y1): (f32, f32), width: f32, color: Rgba) {
    let len = (x1 - x0).hypot(y1 - y0);
    let steps = (len / (width * 0.35)).ceil().max(1.0) as usize;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        batch.circle(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, width / 2.0, color);
    }
}

/// A notification dot at an icon's top right: accent, with a thin dark edge.
pub fn dot(batch: &mut Batch, x: f32, y: f32, size: f32, d: f32) {
    let (cx, cy) = (x + size - 4.0 * d, y + 4.0 * d);
    batch.circle(cx, cy, 6.5 * d, Rgba::hex(BLACK).with_alpha(0.8));
    batch.circle(cx, cy, 5.0 * d, Rgba::hex(accent()));
}

/// A solid triangle `w` wide and `h` tall, its middle at `cy`, pointing
/// right (or left), built from thin upright slices.
pub fn triangle(batch: &mut Batch, x: f32, cy: f32, w: f32, h: f32, right: bool, color: Rgba) {
    const SLICES: usize = 10;
    let sw = w / SLICES as f32;
    for i in 0..SLICES {
        let t = (i as f32 + 0.5) / SLICES as f32;
        let sh = h * if right { 1.0 - t } else { t };
        batch.rect(Rect::new(x + i as f32 * sw, cy - sh / 2.0, sw + 0.5, sh), color, 0.0);
    }
}
