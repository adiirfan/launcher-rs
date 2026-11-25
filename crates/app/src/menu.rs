//! The long-press look: the bar of actions across the top, and the small
//! dark menus (an app's own shortcuts, or Wallpapers and Home settings).

use launcher_core::geometry::Insets;
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::fonts::Weight;
use crate::holo::{self, BLACK, DIVIDER, WHITE};
use crate::render::Renderer;

pub type R4 = (f32, f32, f32, f32);

pub const POPUP_BG: u32 = 0x1C1C1C;
pub const POPUP_BORDER: u32 = 0x3A3A3A;
const POPUP_LINE: u32 = 0x333333;
const ROW_DP: f32 = 52.0;

pub fn inside(r: R4, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// Full width, 64 dp tall, just under the status bar.
pub fn top_bar(screen_w: f32, insets: Insets, d: f32) -> R4 {
    (0.0, insets.top, screen_w, 64.0 * d)
}

/// Which third of the bar (0, 1, 2) is under the finger.
pub fn top_bar_hit(bar: R4, x: f32, y: f32) -> Option<usize> {
    inside(bar, x, y).then(|| (((x - bar.0) / (bar.2 / 3.0)) as usize).min(2))
}

/// Three labels split by thin lines. Greyed-out parts do nothing; the part
/// under a dragged icon lights up in the accent colour.
#[allow(clippy::too_many_arguments)]
pub fn draw_top_bar(gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, bar: R4, labels: [&str; 3], enabled: [bool; 3], hover: Option<usize>, d: f32) {
    let (x, y, w, h) = bar;
    batch.rect(Rect::new(x, y, w, h), Rgba::hex(BLACK), 0.0);
    batch.rect(Rect::new(x, y + h - 1.0 * d, w, 1.0 * d), Rgba::hex(DIVIDER), 0.0);
    let part = w / 3.0;
    let (em, spacing) = (14.0 * d, 0.6 * d);
    let baseline = y + h / 2.0 + r.ascent(em) / 2.0 - 1.0 * d;
    for (i, label) in labels.iter().enumerate() {
        let px = x + part * i as f32;
        if i > 0 {
            batch.rect(Rect::new(px, y + 18.0 * d, 1.0 * d, h - 36.0 * d), Rgba::hex(DIVIDER), 0.0);
        }
        if hover == Some(i) && enabled[i] {
            batch.rect(Rect::new(px, y, part, h - 1.0 * d), Rgba::hex(holo::accent()).with_alpha(0.18), 0.0);
        }
        let color = match (enabled[i], hover == Some(i)) {
            (false, _) => Rgba::hex(WHITE).with_alpha(0.3),
            (true, true) => Rgba::hex(holo::accent()),
            (true, false) => Rgba::hex(WHITE),
        };
        let tw = r.measure_spaced(label, em, spacing, Weight::Medium).min(part - 8.0 * d);
        r.text_spaced(gl, batch, label, px + (part - tw) / 2.0, baseline, em, spacing, part - 8.0 * d, color, Weight::Medium);
    }
}

/// Where a menu of `rows` rows goes beside `anchor` (an icon, or a point
/// with no size): below it when it's in the top half of the screen,
/// above it when lower; lined up with its left edge, or its right edge
/// when that doesn't fit. Always kept on screen.
pub fn popup_rect(anchor: R4, rows: usize, width_dp: f32, screen: (f32, f32), insets: Insets, d: f32) -> R4 {
    let (w, h) = (width_dp * d, rows as f32 * ROW_DP * d + 2.0 * d);
    let gap = 8.0 * d;
    let mut x = anchor.0;
    if x + w > screen.0 - gap {
        x = anchor.0 + anchor.2 - w;
    }
    let x = x.clamp(gap, (screen.0 - gap - w).max(gap));
    let low = anchor.1 + anchor.3 / 2.0 > screen.1 / 2.0;
    let y = if low { anchor.1 - gap - h } else { anchor.1 + anchor.3 + gap };
    let y = y.clamp(insets.top + gap, (screen.1 - insets.bottom - gap - h).max(insets.top + gap));
    (x, y, w, h)
}

fn row_rect(popup: R4, i: usize, d: f32) -> R4 {
    (popup.0, popup.1 + 1.0 * d + i as f32 * ROW_DP * d, popup.2, ROW_DP * d)
}

pub fn popup_hit(popup: R4, rows: usize, x: f32, y: f32, d: f32) -> Option<usize> {
    (0..rows).find(|&i| inside(row_rect(popup, i, d), x, y))
}

/// The dark menu box. `caps` draws the labels as Holo buttons (14 sp
/// medium capitals) instead of menu items (17 sp light).
#[allow(clippy::too_many_arguments)]
pub fn draw_popup(gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, popup: R4, labels: &[String], pressed: Option<usize>, caps: bool, d: f32) {
    let (x, y, w, h) = popup;
    batch.glow(Rect::new(x, y + 4.0 * d, w, h), Rgba::hex(BLACK).with_alpha(0.8), 2.0 * d, 16.0 * d);
    batch.rect(Rect::new(x, y, w, h), Rgba::hex(POPUP_BG), 2.0 * d);
    batch.outline(Rect::new(x, y, w, h), Rgba::hex(POPUP_BORDER), 2.0 * d, 1.0 * d);
    let (em, spacing, weight) = if caps { (14.0 * d, 0.6 * d, Weight::Medium) } else { (17.0 * d, 0.0, Weight::Light) };
    let ascent = r.ascent(em);
    for (i, label) in labels.iter().enumerate() {
        let (rx, ry, rw, rh) = row_rect(popup, i, d);
        if pressed == Some(i) {
            batch.rect(Rect::new(rx + 1.0 * d, ry, rw - 2.0 * d, rh), Rgba::hex(holo::accent()).with_alpha(0.3), 0.0);
        }
        if i + 1 < labels.len() {
            batch.rect(Rect::new(rx + 1.0 * d, ry + rh - 1.0 * d, rw - 2.0 * d, 1.0 * d), Rgba::hex(POPUP_LINE), 0.0);
        }
        let max = rw - 32.0 * d;
        r.text_spaced(gl, batch, label, rx + 16.0 * d, ry + rh / 2.0 + ascent / 2.0 - 1.0 * d, em, spacing, max, Rgba::hex(WHITE), weight);
    }
}
