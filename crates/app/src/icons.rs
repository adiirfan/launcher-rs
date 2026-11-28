//! App icons for every screen: from the atlas, else from the disk cache,
//! else fetched from Android on a background thread (then saved to disk and
//! uploaded straight to the atlas without blocking drawing).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Condvar, LazyLock, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use android_activity::AndroidApp;
use launcher_core::home::AppKey;
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};

use crate::apps;
use crate::cache::Store;
use crate::render::Renderer;

struct IconJobResult {
    key: AppKey,
    size: u32,
    /// `(rgba_pixels, qoi_bytes)` when Android drew the icon; `None` if it couldn't.
    pixels: Option<(Vec<u8>, Option<Vec<u8>>)>,
}

struct WorkerState {
    app: Option<AndroidApp>,
    store: Option<Store>,
    size: u32,
    urgent: VecDeque<AppKey>,
    prefetch: VecDeque<AppKey>,
    queued: HashSet<AppKey>,
    ready: Vec<IconJobResult>,
}

static WORKER: LazyLock<Mutex<WorkerState>> = LazyLock::new(|| {
    Mutex::new(WorkerState {
        app: None,
        store: None,
        size: 0,
        urgent: VecDeque::new(),
        prefetch: VecDeque::new(),
        queued: HashSet::new(),
        ready: Vec::new(),
    })
});
static WORK: Condvar = Condvar::new();
static STARTED: OnceLock<()> = OnceLock::new();

fn run_worker() {
    loop {
        let (app, store, size, key) = {
            let mut s = WORKER.lock().unwrap();
            loop {
                if s.size > 0
                    && let (Some(app), Some(store)) = (s.app.clone(), s.store.clone())
                    && let Some(key) = s.urgent.pop_front().or_else(|| s.prefetch.pop_front())
                {
                    break (app, store, s.size, key);
                }
                s = WORK.wait(s).unwrap();
            }
        };
        let loaded = apps::load_icon_rgba(&app, &key, size as i32);
        let pixels = loaded.map(|(px, w, _)| {
            let qoi = Store::encode_icon(w as u32, &px);
            if let Some(bytes) = qoi.as_deref() {
                store.save_icon_bytes(&key, bytes);
            }
            (px, qoi)
        });
        let mut s = WORKER.lock().unwrap();
        s.queued.remove(&key);
        if s.size == size {
            s.ready.push(IconJobResult { key, size, pixels });
            drop(s);
            crate::keyboard::wake();
        }
    }
}

fn enqueue_worker(key: &AppKey, size: u32, urgent: bool) {
    if size == 0 {
        return;
    }
    let mut s = WORKER.lock().unwrap();
    if s.size != size {
        return;
    }
    if s.queued.insert(key.clone()) {
        if urgent {
            s.urgent.push_back(key.clone());
        } else {
            s.prefetch.push_back(key.clone());
        }
        drop(s);
        WORK.notify_one();
    } else if urgent {
        // Promote a queued background prefetch to urgent when the icon scrolls onto screen.
        if let Some(pos) = s.prefetch.iter().position(|k| k == key) {
            let k = s.prefetch.remove(pos).unwrap();
            s.urgent.push_back(k);
            drop(s);
            WORK.notify_one();
        }
    }
}

pub struct Icons {
    store: Store,
    size: u32,
    /// Icons Android couldn't draw; shown as a plain circle, not retried.
    failed: HashSet<AppKey>,
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
        Icons {
            store,
            size: 0,
            failed: HashSet::new(),
            stale: Vec::new(),
            bytes: HashMap::new(),
            no_file: HashSet::new(),
            spent: Duration::ZERO,
            decoded: 0,
            deferred: false,
            budget: Duration::from_millis(3),
        }
    }

    /// Hand the current activity and store to the background icon worker.
    pub fn start(&self, app: &AndroidApp) {
        {
            let mut s = WORKER.lock().unwrap();
            s.app = Some(app.clone());
            s.store = Some(self.store.clone());
            s.size = self.size;
        }
        STARTED.get_or_init(|| {
            thread::Builder::new().name("icons".into()).spawn(run_worker).expect("icon thread");
        });
        WORK.notify_one();
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    /// Icons are drawn at one size everywhere. A new size means the saved
    /// ones no longer match; they're refetched as they're needed.
    pub fn set_size(&mut self, px: u32) {
        if px != self.size {
            self.size = px;
            self.bytes.clear();
            self.no_file.clear();
            let mut s = WORKER.lock().unwrap();
            s.size = px;
            s.urgent.clear();
            s.prefetch.clear();
            s.queued.clear();
            s.ready.clear();
        }
    }

    /// The app was updated or removed: its icon must be fetched again.
    pub fn changed(&mut self, key: &AppKey) {
        self.store.forget_icon(key);
        self.failed.remove(key);
        self.bytes.remove(key);
        self.no_file.remove(key);
        self.stale.push(key.clone());
        let mut s = WORKER.lock().unwrap();
        s.ready.retain(|item| &item.key != key);
    }

    pub fn begin_frame(&mut self, r: &mut Renderer) {
        self.spent = Duration::ZERO;
        self.decoded = 0;
        self.deferred = false;
        for key in self.stale.drain(..) {
            r.forget_icon(&atlas_key(&key));
        }
    }

    /// Upload icons finished by the background worker straight into the atlas.
    /// Returns true if any new icon became ready (so a redraw should show it).
    pub fn drain_ready(&mut self, gl: &glow::Context, r: &mut Renderer) -> bool {
        let ready = {
            let mut s = WORKER.lock().unwrap();
            if s.ready.is_empty() {
                return false;
            }
            std::mem::take(&mut s.ready)
        };
        let mut uploaded = false;
        for item in ready {
            if item.size != self.size {
                continue;
            }
            match item.pixels {
                Some((rgba, qoi)) => {
                    self.no_file.remove(&item.key);
                    self.failed.remove(&item.key);
                    if let Some(bytes) = qoi {
                        self.bytes.insert(item.key.clone(), bytes);
                    }
                    let ak = atlas_key(&item.key);
                    r.icon(gl, &ak, &rgba, item.size, item.size);
                    uploaded = true;
                }
                None => {
                    self.failed.insert(item.key);
                }
            }
        }
        uploaded
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
                // Either missing on disk or cached at a different size: fetch asynchronously.
                self.bytes.remove(key);
                self.no_file.insert(key.clone());
                enqueue_worker(key, self.size, true);
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
    /// Returns the previous budget so a caller can temporarily lift it.
    pub fn set_frame_budget(&mut self, budget: Duration) -> Duration {
        std::mem::replace(&mut self.budget, budget)
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

    /// Queue any apps that do not yet have a cached icon on disk so the
    /// background worker builds them ahead of opening the app list.
    pub fn prefetch_missing<'a>(&mut self, apps: impl IntoIterator<Item = &'a AppKey>) {
        if self.size == 0 {
            return;
        }
        for key in apps {
            if self.no_file.contains(key) && !self.failed.contains(key) {
                enqueue_worker(key, self.size, false);
            }
        }
    }

    /// Put icons into the atlas ahead of being drawn, until `budget` is used
    /// or the atlas has no free slot. True while some are left. Call only
    /// while nothing moves, after `warm` has read their files.
    pub fn preload<'a>(&mut self, gl: &glow::Context, r: &mut Renderer, apps: impl IntoIterator<Item = &'a AppKey>, budget: Duration) -> bool {
        if self.size == 0 {
            return false;
        }
        let t = Instant::now();
        let mut wrong_size = Vec::new();
        let mut more = false;
        for key in apps {
            let ak = atlas_key(key);
            if r.icon_uv(&ak).is_some() || self.failed.contains(key) {
                continue;
            }
            let Some(bytes) = self.bytes.get(key) else { continue };
            if !r.icon_room() {
                break;
            }
            if t.elapsed() >= budget {
                more = true;
                break;
            }
            match Store::decode_icon(bytes, self.size) {
                Some((px, w, h)) => {
                    r.icon(gl, &ak, &px, w, h);
                }
                None => {
                    wrong_size.push(key.clone());
                }
            }
        }
        for key in wrong_size {
            self.bytes.remove(&key);
            self.no_file.insert(key.clone());
            enqueue_worker(&key, self.size, false);
        }
        more
    }

    /// Wait for queued uploads to finish, so the graphics driver can reuse
    /// its temporary copies instead of piling up more (seen on Adreno).
    pub fn settle(gl: &glow::Context) {
        use glow::HasContext;
        unsafe { gl.finish() };
    }

    /// Drop compressed copies for icons already resident in the GPU atlas.
    pub fn trim_uploaded(&mut self, r: &mut Renderer) {
        self.bytes.retain(|k, _| r.icon_uv(&atlas_key(k)).is_none());
    }

    /// Give back all compressed copies; they're read again when needed.
    pub fn trim(&mut self) {
        self.bytes = HashMap::new();
        self.no_file = HashSet::new();
    }
}
