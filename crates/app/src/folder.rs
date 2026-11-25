//! Folders: the round icon on Home, and the panel that opens when tapped.

use std::collections::{HashMap, HashSet};

use launcher_core::geometry::COLUMN_MAX_DP;
use launcher_core::home::{AppKey, Folder};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::fonts::Weight;
use crate::holo::{self, BLACK, PLACEHOLDER, WHITE};
use crate::icons::Icons;
use crate::menu::{POPUP_BG, POPUP_BORDER, R4};
use crate::render::Renderer;

const COLS: usize = 4;

/// A dark circle with a white ring and up to four of its apps inside.
#[allow(clippy::too_many_arguments)]
pub fn draw_icon(gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, icons: &mut Icons, folder: &Folder, (x, y, s): (f32, f32, f32), pressed: bool, d: f32) {
    let fade = if pressed { 0.7 } else { 1.0 };
    let (cx, cy, rad) = (x + s / 2.0, y + s / 2.0, s / 2.0);
    batch.circle(cx, cy, rad, Rgba::hex(BLACK).with_alpha(0.6 * fade));
    batch.outline(Rect::new(x, y, s, s), Rgba::hex(WHITE).with_alpha(0.9 * fade), rad, 2.0 * d);
    let small = s * 0.3;
    let gap = s * 0.04;
    // Two apps sit in one row across the middle; three or four in two rows.
    let rows = folder.apps.len().min(4).div_ceil(2) as f32;
    let start = (cx - small - gap / 2.0, cy - (rows * small + (rows - 1.0) * gap) / 2.0);
    for (i, key) in folder.apps.iter().take(4).enumerate() {
        let (col, row) = ((i % 2) as f32, (i / 2) as f32);
        let at = (start.0 + col * (small + gap), start.1 + row * (small + gap), small);
        icons.draw(gl, r, batch, key, at, false, fade);
    }
}

/// Where everything in an open folder sits.
pub struct Panel {
    pub panel: R4,
    pub name: R4,
    icon: f32,
    cell_w: f32,
    row_h: f32,
    grid_top: f32,
    grid_left: f32,
}

impl Panel {
    /// Full width less 16 dp each side, starting at `top`; on a wide screen
    /// no wider than Home's centred column.
    pub fn new(screen_w: f32, top: f32, icon: f32, apps: usize, d: f32) -> Panel {
        let w = (screen_w - 32.0 * d).min((COLUMN_MAX_DP - 32.0) * d);
        let x = (screen_w - w) / 2.0;
        let name = (x + 16.0 * d, top + 8.0 * d, w - 32.0 * d, 48.0 * d);
        let grid_top = name.1 + name.3 + 8.0 * d;
        let row_h = icon + 6.0 * d + 18.0 * d + 14.0 * d;
        let rows = apps.div_ceil(COLS).max(1);
        let h = grid_top - top + rows as f32 * row_h + 8.0 * d;
        Panel { panel: (x, top, w, h), name, icon, cell_w: (w - 16.0 * d) / COLS as f32, row_h, grid_top, grid_left: x + 8.0 * d }
    }

    pub fn icon_rect(&self, i: usize) -> R4 {
        let (col, row) = ((i % COLS) as f32, (i / COLS) as f32);
        let cx = self.grid_left + col * self.cell_w + (self.cell_w - self.icon) / 2.0;
        (cx, self.grid_top + row * self.row_h, self.icon, self.icon)
    }

    /// The app cell (icon and label) under the finger.
    pub fn hit(&self, x: f32, y: f32, apps: usize) -> Option<usize> {
        (0..apps).find(|&i| {
            let (ix, iy, s, _) = self.icon_rect(i);
            let pad = (self.cell_w - s) / 2.0;
            x >= ix - pad && x < ix + s + pad && y >= iy && y < iy + self.row_h
        })
    }

    /// The panel, the folder's name (or the text being typed for a new
    /// one, with a caret), and its apps with their labels.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, icons: &mut Icons, folder: &Folder, labels: &HashMap<AppKey, String>, dots: Option<&HashSet<String>>, renaming: Option<&str>, pressed: Option<usize>, d: f32) {
        let (x, y, w, h) = self.panel;
        batch.glow(Rect::new(x, y + 4.0 * d, w, h), Rgba::hex(BLACK).with_alpha(0.8), 2.0 * d, 16.0 * d);
        batch.rect(Rect::new(x, y, w, h), Rgba::hex(POPUP_BG), 2.0 * d);
        batch.outline(Rect::new(x, y, w, h), Rgba::hex(POPUP_BORDER), 2.0 * d, 1.0 * d);

        let (nx, ny, nw, nh) = self.name;
        let em = 18.0 * d;
        let baseline = ny + nh / 2.0 + r.ascent(em) / 2.0 - 1.0 * d;
        match renaming {
            Some(text) => {
                let (shown, color) = if text.is_empty() { (folder.name.as_str(), Rgba::hex(PLACEHOLDER)) } else { (text, Rgba::hex(WHITE)) };
                let tw = if text.is_empty() { 0.0 } else { r.measure(shown, em, nw, Weight::Light) };
                r.text(gl, batch, shown, nx, baseline, em, nw, color, Weight::Light);
                batch.rect(Rect::new(nx + tw + 1.0 * d, ny + 12.0 * d, 2.0 * d, nh - 24.0 * d), Rgba::hex(holo::accent()), 0.0);
                batch.rect(Rect::new(nx, ny + nh - 2.0 * d, nw, 2.0 * d), Rgba::hex(holo::accent()), 0.0);
            }
            None => {
                r.text(gl, batch, &folder.name, nx, baseline, em, nw, Rgba::hex(WHITE), Weight::Light);
                batch.rect(Rect::new(nx, ny + nh - 1.0 * d, nw, 1.0 * d), Rgba::hex(POPUP_BORDER), 0.0);
            }
        }

        let label_em = 13.0 * d;
        let ascent = r.ascent(label_em);
        let label_max = self.cell_w - 8.0 * d;
        for (i, key) in folder.apps.iter().enumerate() {
            let (ix, iy, s, _) = self.icon_rect(i);
            icons.draw(gl, r, batch, key, (ix, iy, s), pressed == Some(i), 1.0);
            if dots.is_some_and(|set| set.contains(&key.package)) {
                holo::dot(batch, ix, iy, s, d);
            }
            let label = labels.get(key).map_or("", String::as_str);
            let lw = r.measure(label, label_em, label_max, Weight::Regular);
            r.text(gl, batch, label, ix + s / 2.0 - lw / 2.0, iy + s + 6.0 * d + ascent, label_em, label_max, Rgba::hex(WHITE), Weight::Regular);
        }
    }
}
