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
