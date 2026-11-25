//! App icons for every screen: from the atlas, else from the disk cache,
//! else asked from Android after the frame (then saved to disk).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

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
    /// Cached icon files held compressed in memory (about 20 KB each), read
    /// while idle so drawing never waits on storage.
    bytes: HashMap<AppKey, Vec<u8>>,
    /// Keys whose file isn't there (Android must be asked instead).
    no_file: HashSet<AppKey>,
    /// Time spent this frame putting icons into the atlas, and how many.
    pub spent: Duration,
    pub decoded: u32,
    /// An icon was left as a placeholder to keep this frame on time.
    deferred: bool,
    /// Icon work allowed per frame; the rest waits for the next frame.
    budget: Duration,
}

fn atlas_key(key: &AppKey) -> String {
    format!("{}/{}", key.package, key.activity)
}

impl Icons {
    pub fn new(store: Store) -> Icons {
        Icons { store, size: 0, failed: HashSet::new(), missing: Vec::new(), stale: Vec::new(),
            bytes: HashMap::new(), no_file: HashSet::new(), spent: Duration::ZERO, decoded: 0, deferred: false,
            budget: Duration::from_millis(3) }
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
            self.bytes.clear();
            self.no_file.clear();
        }
    }

    /// The app was updated or removed: its icon must be fetched again.
    pub fn changed(&mut self, key: &AppKey) {
        self.store.forget_icon(key);
        self.failed.remove(key);
        self.bytes.remove(key);
        self.no_file.remove(key);
        self.stale.push(key.clone());
    }

    pub fn begin_frame(&mut self, r: &mut Renderer) {
        self.spent = Duration::ZERO;
        self.decoded = 0;
        self.deferred = false;
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
        if self.spent >= self.budget {
            self.deferred = true;
            return None;
        }
        let t = Instant::now();
        if !self.bytes.contains_key(key) && !self.no_file.contains(key) {
            self.read_file(key);
        }
        let loaded = self.bytes.get(key).and_then(|b| Store::decode_icon(b, self.size));
        let uv = loaded.as_ref().and_then(|(px, w, h)| r.icon(gl, &ak, px, *w, *h));
        self.spent += t.elapsed();
        self.decoded += 1;
        match loaded {
            Some(_) => uv,
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

    /// Ask Android for one missing icon and save it. Each takes 10-60 ms,
    /// so one per call, and only while nothing moves. True if one was tried.
    pub fn load_missing(&mut self, app: &AndroidApp) -> bool {
        let Some(key) = self.missing.pop() else { return false };
        log::info!("icon from Android ({} more waiting)", self.missing.len());
        match apps::load_icon_rgba(app, &key, self.size as i32) {
            Some((px, w, _)) => {
                self.store.save_icon(&key, w as u32, &px);
                // Any copy held is the old (unusable) file: read the new one.
                self.bytes.remove(&key);
                self.no_file.remove(&key);
            }
            None => {
                self.failed.insert(key);
            }
        }
        true
    }

    fn read_file(&mut self, key: &AppKey) {
        match self.store.read_icon(key) {
            Some(b) => {
                self.bytes.insert(key.clone(), b);
            }
            None => {
                self.no_file.insert(key.clone());
            }
        }
    }

    /// A fifth of a refresh suits most phones: 3.3 ms at 60 Hz, 1.7 at 120.
    pub fn set_frame_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    /// True when an icon was left as a placeholder to stay on time.
    pub fn deferred(&self) -> bool {
        self.deferred
    }

    /// Read cached icon files into memory until `budget` is used. True while
    /// some are left. Call only while nothing moves.
    pub fn warm<'a>(&mut self, apps: impl IntoIterator<Item = &'a AppKey>, budget: Duration) -> bool {
        if self.size == 0 {
            return false;
        }
        let t = Instant::now();
        for key in apps {
            if self.bytes.contains_key(key) || self.no_file.contains(key) || self.failed.contains(key) {
                continue;
            }
            if t.elapsed() >= budget {
                return true;
            }
            self.read_file(key);
        }
        false
    }

    /// Put icons into the atlas ahead of being drawn, until `budget` is used
    /// or the atlas has no free slot. True while some are left. Call only
    /// while nothing moves, after `warm` has read their files.
    pub fn preload<'a>(&mut self, gl: &glow::Context, r: &mut Renderer, apps: impl IntoIterator<Item = &'a AppKey>, budget: Duration) -> bool {
        if self.size == 0 {
            return false;
        }
        let t = Instant::now();
        for key in apps {
            let ak = atlas_key(key);
            if r.icon_uv(&ak).is_some() || self.failed.contains(key) {
                continue;
            }
            let Some(bytes) = self.bytes.get(key) else { continue };
            if !r.icon_room() {
                return false;
            }
            if t.elapsed() >= budget {
                return true;
            }
            if let Some((px, w, h)) = Store::decode_icon(bytes, self.size) {
                r.icon(gl, &ak, &px, w, h);
            }
        }
        false
    }

    /// Give back the compressed copies; they're read again when needed.
    pub fn trim(&mut self) {
        self.bytes = HashMap::new();
        self.no_file = HashSet::new();
    }
}
