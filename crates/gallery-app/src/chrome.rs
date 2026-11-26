//! Holo pieces every gallery screen shares: the action bar and its icon.

use glow::Context;
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, BAR, BLACK, WHITE};
use holo_platform::render::Renderer;
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

/// Height of the action bar.
pub const BAR_DP: f32 = 56.0;

/// The app's own bar: accent square with a picture, and a title. `top` is
/// where the bar starts (under the status bar). Returns the bar's bottom.
pub fn action_bar(gl: &Context, r: &mut Renderer, batch: &mut Batch, w: f32, top: f32, title: &str, d: f32) -> f32 {
    let h = BAR_DP * d;
    batch.rect(Rect::new(0.0, 0.0, w, top + h), Rgba::hex(BAR), 0.0);
    let cy = top + h / 2.0;
    let s = 32.0 * d;
    let (sx, sy) = (12.0 * d, cy - s / 2.0);
    batch.rect(Rect::new(sx, sy, s, s), Rgba::hex(holo::accent()), 4.0 * d);
    picture_glyph(batch, sx + 6.0 * d, sy + 6.0 * d, 20.0 * d, Rgba::hex(BLACK), d);
    let em = 20.0 * d;
    r.text(gl, batch, title, sx + s + 10.0 * d, cy + r.ascent(em) / 2.0 - 1.0 * d, em, w - 120.0 * d, Rgba::hex(WHITE), Weight::Regular);
    top + h
}

/// A framed picture with two hills and a sun, `size` square.
pub fn picture_glyph(batch: &mut Batch, x: f32, y: f32, size: f32, c: Rgba, d: f32) {
    let u = size / 24.0;
    let lw = 1.6 * u;
    batch.outline(Rect::new(x + 4.0 * u, y + 5.0 * u, 16.0 * u, 14.0 * u), c, 0.0, lw);
    holo::stroke(batch, (x + 4.0 * u, y + 16.0 * u), (x + 9.0 * u, y + 11.0 * u), lw, c);
    holo::stroke(batch, (x + 9.0 * u, y + 11.0 * u), (x + 13.0 * u, y + 15.0 * u), lw, c);
    holo::stroke(batch, (x + 13.0 * u, y + 15.0 * u), (x + 16.0 * u, y + 12.0 * u), lw, c);
    holo::stroke(batch, (x + 16.0 * u, y + 12.0 * u), (x + 20.0 * u, y + 16.0 * u), lw, c);
    batch.circle(x + 15.5 * u, y + 8.5 * u, 1.3 * u, c);
}
