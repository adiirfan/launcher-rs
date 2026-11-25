//! App icons for every screen: from the atlas, else from the disk cache,
//! else asked from Android after the frame (then saved to disk).

use std::collections::HashSet;

use android_activity::AndroidApp;
use launcher_core::home::AppKey;
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::apps;
use crate::cache::Store;
use crate::render::Renderer;

pub struct Icons {
    store: Store,
    size: u32,
    /// Icons Android couldn't draw; shown as a plain circle, not retried.
    failed: HashSet<AppKey>,
    /// Icons not on disk yet: fetched from Android after the frame.
    missing: Vec<AppKey>,
    /// Icons whose app changed: dropped from the atlas next frame.
    stale: Vec<AppKey>,
}

fn atlas_key(key: &AppKey) -> String {
    format!("{}/{}", key.package, key.activity)
}

impl Icons {
    pub fn new(store: Store) -> Icons {
        Icons { store, size: 0, failed: HashSet::new(), missing: Vec::new(), stale: Vec::new() }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    /// Icons are drawn at one size everywhere. A new size means the saved
    /// ones no longer match; they're refetched as they're needed.
    pub fn set_size(&mut self, px: u32) {
        if px != self.size {
            self.size = px;
            self.missing.clear();
        }
    }

    /// The app was updated or removed: its icon must be fetched again.
    pub fn changed(&mut self, key: &AppKey) {
        self.store.forget_icon(key);
        self.failed.remove(key);
        self.stale.push(key.clone());
    }

    pub fn begin_frame(&mut self, r: &mut Renderer) {
        for key in self.stale.drain(..) {
            r.forget_icon(&atlas_key(&key));
        }
    }

    fn uv(&mut self, gl: &glow::Context, r: &mut Renderer, key: &AppKey) -> Option<[f32; 4]> {
        let ak = atlas_key(key);
        if let Some(uv) = r.icon_uv(&ak) {
            return Some(uv);
        }
        if self.failed.contains(key) {
            return None;
        }
        match self.store.load_icon(key, self.size) {
            Some((px, w, h)) => r.icon(gl, &ak, &px, w, h),
            None => {
                if !self.missing.contains(key) {
                    self.missing.push(key.clone());
                }
                None
            }
        }
    }

    /// Queue an icon at (x, y) with edge `s`. Pressed icons shrink a little
    /// and dim; `fade` scales opacity (for screens sliding in or out).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, gl: &glow::Context, r: &mut Renderer, batch: &mut Batch, key: &AppKey, (x, y, s): (f32, f32, f32), pressed: bool, fade: f32) {
        let (mut x, mut y, mut s) = (x, y, s);
        let mut opacity = fade;
        if pressed {
            let shrunk = s * 0.92;
            x += (s - shrunk) / 2.0;
            y += (s - shrunk) / 2.0;
            s = shrunk;
            opacity *= 0.7;
        }
        match self.uv(gl, r, key) {
            // Shapes are cut into the pixels already; no mask here.
            Some(uv) => batch.image(Rect::new(x, y, s, s), uv, 0.0, opacity),
            None => batch.rect(Rect::new(x, y, s, s), Rgba::hex(0x737373).with_alpha(opacity), s * 0.18),
        }
    }

    pub fn has_missing(&self) -> bool {
        !self.missing.is_empty()
    }

    /// Fetch icons that weren't on disk and save them. True if any were tried.
    pub fn load_missing(&mut self, app: &AndroidApp) -> bool {
        let missing = std::mem::take(&mut self.missing);
        for key in &missing {
            match apps::load_icon_rgba(app, key, self.size as i32) {
                Some((px, w, _)) => self.store.save_icon(key, w as u32, &px),
                None => {
                    self.failed.insert(key.clone());
                }
            }
        }
        !missing.is_empty()
    }
}
