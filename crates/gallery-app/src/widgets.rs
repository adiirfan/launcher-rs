//! Holo pieces the gallery's screens are built from, drawn to match the
//! approved mockup: tab strip, sort spinner, overflow button, section
//! heading, popup menu with radio rows, badges and buttons.

use glow::Context;
use holo_platform::fonts::Weight;
use holo_platform::holo::{self, BAR, BLACK, DIVIDER, TEXT_QUIET, WHITE};
use holo_platform::render::Renderer;
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

pub const TABS_DP: f32 = 48.0;
pub const MENU_ROW_DP: f32 = 48.0;
const POPUP_BG: u32 = 0x222222;
const POPUP_BORDER: u32 = 0x3A3A3A;
const POPUP_LINE: u32 = 0x2E2E2E;
const TAB_OFF: u32 = 0x8A8A8A;
pub const TILE_EMPTY: u32 = 0x1A1A1A;

fn accent() -> Rgba {
    Rgba::hex(holo::accent())
}

/// Baseline that centres one line of `em` text on `cy`.
pub fn baseline(r: &Renderer, cy: f32, em: f32, d: f32) -> f32 {
    cy + r.ascent(em) / 2.0 - 1.0 * d
}

/// Two tabs under the action bar; the chosen one bright with an accent bar.
pub fn tabs(gl: &Context, r: &mut Renderer, batch: &mut Batch, x: f32, y: f32, w: f32, labels: [&str; 2], chosen: usize, d: f32) {
    let h = TABS_DP * d;
    batch.rect(Rect::new(x, y, w, h), Rgba::hex(BAR), 0.0);
    batch.rect(Rect::new(x, y + h - 1.0 * d, w, 1.0 * d), Rgba::hex(DIVIDER), 0.0);
    let half = w / 2.0;
    batch.rect(Rect::new(x + half - 0.5 * d, y + 14.0 * d, 1.0 * d, h - 28.0 * d), Rgba::hex(DIVIDER), 0.0);
    let em = 14.0 * d;
    for (i, label) in labels.iter().enumerate() {
        let tx = x + half * i as f32;
        let lw = r.measure_spaced(label, em, 0.6 * d, Weight::Medium);
        let color = if i == chosen { Rgba::hex(WHITE) } else { Rgba::hex(TAB_OFF) };
        r.text_spaced(gl, batch, label, tx + (half - lw) / 2.0, baseline(r, y + h / 2.0 - 2.0 * d, em, d), em, 0.6 * d, half, color, Weight::Medium);
        if i == chosen {
            batch.rect(Rect::new(tx, y + h - 4.0 * d, half, 4.0 * d), accent(), 0.0);
        }
    }
}

/// "SORT" over the chosen sort, with Holo's little corner triangle.
/// `right` is its right edge; returns its left edge.
pub fn spinner(gl: &Context, r: &mut Renderer, batch: &mut Batch, right: f32, cy: f32, caption: &str, value: &str, d: f32) -> f32 {
    let (small, big) = (11.0 * d, 14.0 * d);
    let w = r.measure_spaced(value, big, 0.4 * d, Weight::Medium).max(r.measure_spaced(caption, small, 0.4 * d, Weight::Regular));
    let x = right - 18.0 * d - w;
    r.text_spaced(gl, batch, caption, x, cy - 3.0 * d, small, 0.4 * d, w + 1.0, Rgba::hex(TEXT_QUIET), Weight::Regular);
    r.text_spaced(gl, batch, value, x, cy + 14.0 * d, big, 0.4 * d, w + 1.0, Rgba::hex(WHITE), Weight::Medium);
    // The triangle in the bottom right corner.
    let s = 8.0 * d;
    let (x0, y0) = (right - 6.0 * d - s, cy + 15.0 * d - s);
    let n = 8;
    for i in 0..n {
        let f = (i + 1) as f32 / n as f32;
        batch.rect(Rect::new(x0 + s * (1.0 - f), y0 + s * i as f32 / n as f32, s * f, s / n as f32 + 0.5), Rgba::hex(TEXT_QUIET), 0.0);
    }
    x - 8.0 * d
}

/// A small accent triangle in a tile's top-left corner: an edited copy.
pub fn edited_corner(batch: &mut Batch, x: f32, y: f32, size: f32) {
    let n = 10;
    for i in 0..n {
        let f = 1.0 - i as f32 / n as f32;
        batch.rect(Rect::new(x, y + size * i as f32 / n as f32, size * f, size / n as f32 + 0.5), accent(), 0.0);
    }
}

/// Three dots, centred on `cx, cy`.
pub fn overflow(batch: &mut Batch, cx: f32, cy: f32, d: f32) {
    for k in [-7.0, 0.0, 7.0] {
        batch.circle(cx, cy + k * d, 1.8 * d, Rgba::hex(WHITE));
    }
}

/// A section heading: accent words with a thin accent line under them,
/// and quiet words on the right (a count).
pub fn heading(gl: &Context, r: &mut Renderer, batch: &mut Batch, x: f32, y: f32, w: f32, label: &str, right: &str, d: f32) {
    let em = 14.0 * d;
    let base = y + 15.0 * d;
    r.text_spaced(gl, batch, label, x + 4.0 * d, base, em, 0.6 * d, w * 0.7, accent(), Weight::Bold);
    if !right.is_empty() {
        let rem = 13.0 * d;
        let rw = r.measure(right, rem, w * 0.3, Weight::Regular);
        r.text(gl, batch, right, x + w - 4.0 * d - rw, base, rem, w * 0.3, Rgba::hex(TEXT_QUIET), Weight::Regular);
    }
    batch.rect(Rect::new(x, y + 22.0 * d, w, 1.0 * d), accent(), 0.0);
}

/// One row of a popup menu.
pub enum MenuRow<'a> {
    /// Small accent caption.
    Caption(&'a str),
    /// A choice with a radio circle; `true` = chosen.
    Radio(&'a str, bool),
    /// A plain command.
    Plain(&'a str),
}

impl MenuRow<'_> {
    pub fn height(&self, d: f32) -> f32 {
        match self {
            MenuRow::Caption(_) => 34.0 * d,
            _ => MENU_ROW_DP * d,
        }
    }
}

/// A popup panel at `x, y` of width `w`. Returns each row's top and
/// height, for hit testing.
pub fn popup(gl: &Context, r: &mut Renderer, batch: &mut Batch, x: f32, y: f32, w: f32, rows: &[MenuRow], pressed: Option<usize>, d: f32) -> Vec<(f32, f32)> {
    let pad = 4.0 * d;
    let h: f32 = rows.iter().map(|row| row.height(d)).sum::<f32>() + 2.0 * pad;
    batch.glow(Rect::new(x, y + 6.0 * d, w, h), Rgba::hex(BLACK).with_alpha(0.8), 0.0, 16.0 * d);
    batch.rect(Rect::new(x, y, w, h), Rgba::hex(POPUP_BG), 0.0);
    batch.outline(Rect::new(x, y, w, h), Rgba::hex(POPUP_BORDER), 0.0, 1.0 * d);
    let mut out = Vec::with_capacity(rows.len());
    let mut ry = y + pad;
    for (i, row) in rows.iter().enumerate() {
        let rh = row.height(d);
        out.push((ry, rh));
        if pressed == Some(i) {
            batch.rect(Rect::new(x + 1.0 * d, ry, w - 2.0 * d, rh), accent().with_alpha(0.3), 0.0);
        }
        match row {
            MenuRow::Caption(text) => {
                if i > 0 {
                    batch.rect(Rect::new(x, ry, w, 1.0 * d), Rgba::hex(POPUP_BORDER), 0.0);
                }
                r.text_spaced(gl, batch, text, x + 16.0 * d, ry + 24.0 * d, 12.0 * d, 0.6 * d, w, accent(), Weight::Bold);
            }
            MenuRow::Radio(text, on) => {
                batch.rect(Rect::new(x + 1.0 * d, ry, w - 2.0 * d, 1.0 * d), Rgba::hex(POPUP_LINE), 0.0);
                let em = 16.0 * d;
                r.text(gl, batch, text, x + 16.0 * d, baseline(r, ry + rh / 2.0, em, d), em, w - 60.0 * d, Rgba::hex(WHITE), Weight::Regular);
                radio(batch, x + w - 23.0 * d, ry + rh / 2.0, *on, d);
            }
            MenuRow::Plain(text) => {
                if i > 0 {
                    batch.rect(Rect::new(x + 1.0 * d, ry, w - 2.0 * d, 1.0 * d), Rgba::hex(POPUP_LINE), 0.0);
                }
                let em = 16.0 * d;
                r.text(gl, batch, text, x + 16.0 * d, baseline(r, ry + rh / 2.0, em, d), em, w - 32.0 * d, Rgba::hex(WHITE), Weight::Regular);
            }
        }
        ry += rh;
    }
    out
}

/// Holo radio button: grey ring, accent dot when chosen.
pub fn radio(batch: &mut Batch, cx: f32, cy: f32, on: bool, d: f32) {
    batch.outline(Rect::new(cx - 8.0 * d, cy - 8.0 * d, 16.0 * d, 16.0 * d), Rgba::hex(0x8C8C8C), 8.0 * d, 2.0 * d);
    if on {
        batch.circle(cx, cy, 4.5 * d, accent());
    }
}

/// A dark label in a tile's corner: a video's length (with a play
/// triangle) or "GIF". `right, bottom` is the tile's corner.
pub fn badge(gl: &Context, r: &mut Renderer, batch: &mut Batch, right: f32, bottom: f32, text: &str, play: bool, d: f32) {
    let em = 11.0 * d;
    let tw = r.measure(text, em, 100.0 * d, Weight::Medium);
    let tri = if play { 11.0 * d } else { 0.0 };
    let (w, h) = (tw + tri + 10.0 * d, 16.0 * d);
    let (x, y) = (right - 4.0 * d - w, bottom - 4.0 * d - h);
    batch.rect(Rect::new(x, y, w, h), Rgba::hex(BLACK).with_alpha(0.72), 0.0);
    if play {
        play_triangle(batch, x + 5.0 * d, y + h / 2.0, 7.0 * d, Rgba::hex(WHITE));
    }
    r.text(gl, batch, text, x + 5.0 * d + tri, baseline(r, y + h / 2.0, em, d), em, tw + 1.0, Rgba::hex(WHITE), Weight::Medium);
}

/// A right-pointing triangle `size` tall, its left edge at `x`.
pub fn play_triangle(batch: &mut Batch, x: f32, cy: f32, size: f32, c: Rgba) {
    let steps = (size.max(4.0)) as usize;
    for i in 0..steps {
        let f = i as f32 / steps as f32;
        let hh = size / 2.0 * (1.0 - f);
        batch.rect(Rect::new(x + f * size * 0.9, cy - hh, size * 0.9 / steps as f32 + 0.5, hh * 2.0), c, 0.0);
    }
}

/// A Holo button: dark grey block with centred caps.
pub fn button(gl: &Context, r: &mut Renderer, batch: &mut Batch, rect: Rect, label: &str, pressed: bool, d: f32) {
    let bg = if pressed { accent().with_alpha(0.45) } else { Rgba::hex(0x2E2E2E) };
    batch.rect(rect, bg, 2.0 * d);
    let em = 14.0 * d;
    let lw = r.measure_spaced(label, em, 0.6 * d, Weight::Medium);
    r.text_spaced(gl, batch, label, rect.x + (rect.w - lw) / 2.0, baseline(r, rect.y + rect.h / 2.0, em, d), em, 0.6 * d, rect.w, Rgba::hex(WHITE), Weight::Medium);
}

/// The corner checkbox on a tile in selection mode: a dark square with a
/// light edge, filled with the accent and a tick when chosen.
pub fn checkbox(batch: &mut Batch, x: f32, y: f32, size: f32, on: bool) {
    let u = size / 30.0;
    let b = Rect::new(x + 5.0 * u, y + 7.0 * u, 18.0 * u, 18.0 * u);
    if on {
        batch.rect(b, accent(), 0.0);
        holo::stroke(batch, (b.x + 4.0 * u, b.y + 9.0 * u), (b.x + 8.0 * u, b.y + 13.0 * u), 2.2 * u, Rgba::hex(WHITE));
        holo::stroke(batch, (b.x + 8.0 * u, b.y + 13.0 * u), (b.x + 15.0 * u, b.y + 5.0 * u), 2.2 * u, Rgba::hex(WHITE));
    } else {
        batch.rect(b, Rgba::hex(BLACK).with_alpha(0.55), 0.0);
        batch.outline(b, Rgba::hex(0xE0E0E0), 0.0, 2.0 * u);
    }
}

fn lines(batch: &mut Batch, pts: &[&[(f32, f32)]], x: f32, y: f32, u: f32, c: Rgba) {
    for p in pts {
        for w in p.windows(2) {
            holo::stroke(batch, (x + w[0].0 * u, y + w[0].1 * u), (x + w[1].0 * u, y + w[1].1 * u), 1.8 * u, c);
        }
    }
}

/// The bottom-bar icons shared by the viewer, selection and Bin: SHARE,
/// DELETE (and DELETE FOREVER), RESTORE. On a 24-unit square.
pub fn action_icon(batch: &mut Batch, which: &str, x: f32, y: f32, size: f32, c: Rgba) {
    let u = size / 24.0;
    match which {
        "SHARE" => {
            for (cx, cy) in [(18.0, 7.0), (6.0, 12.0), (18.0, 17.0)] {
                batch.outline(Rect::new(x + (cx - 2.0) * u, y + (cy - 2.0) * u, 4.0 * u, 4.0 * u), c, 2.0 * u, 1.8 * u);
            }
            lines(batch, &[&[(8.0, 11.0), (16.0, 7.6)], &[(8.0, 13.0), (16.0, 16.4)]], x, y, u, c);
        }
        "MOVE" => {
            // A folder with an arrow going up into it.
            lines(batch, &[&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (21.0, 8.0), (21.0, 19.0), (3.0, 19.0), (3.0, 6.0)], &[(12.0, 11.0), (12.0, 16.0)], &[(9.5, 13.5), (12.0, 11.0), (14.5, 13.5)]], x, y, u, c);
        }
        "CONVERT" => lines(batch, &[&[(4.0, 8.0), (17.0, 8.0)], &[(14.0, 5.0), (17.0, 8.0)], &[(20.0, 16.0), (7.0, 16.0)], &[(10.0, 19.0), (7.0, 16.0)]], x, y, u, c),
        "RESTORE" => {
            // An arrow curling back up: a box with an arrow out of its top.
            lines(batch, &[&[(5.0, 11.0), (5.0, 20.0), (19.0, 20.0), (19.0, 11.0)], &[(12.0, 16.0), (12.0, 4.0)], &[(8.0, 8.0), (12.0, 4.0), (16.0, 8.0)]], x, y, u, c);
        }
        _ => lines(batch, &[&[(5.0, 7.0), (19.0, 7.0)], &[(9.0, 7.0), (9.0, 4.0), (15.0, 4.0), (15.0, 7.0)], &[(7.0, 7.0), (8.0, 20.0), (16.0, 20.0), (17.0, 7.0)]], x, y, u, c),
    }
}

/// A short message in a dark pill, centred, its bottom `gap` above `above`.
pub fn toast(gl: &Context, r: &mut Renderer, batch: &mut Batch, msg: &str, w: f32, above: f32, d: f32) {
    let em = 14.0 * d;
    let mw = r.measure(msg, em, w, Weight::Regular);
    let (tw, h) = (mw + 40.0 * d, 40.0 * d);
    let rc = Rect::new((w - tw) / 2.0, above - h - 24.0 * d, tw, h);
    batch.rect(rc, Rgba::hex(0x333333).with_alpha(0.95), 4.0 * d);
    r.text(gl, batch, msg, rc.x + 20.0 * d, baseline(r, rc.y + h / 2.0, em, d), em, mw + 1.0, Rgba::hex(WHITE), Weight::Regular);
}
