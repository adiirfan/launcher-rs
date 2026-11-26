//! Thumbnails for the grid: two threads ask Android for them, the main loop
//! puts the finished ones into the renderer's picture slots a few at a time.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use android_activity::AndroidApp;
use gallery_core::thumbq::ThumbQueue;
use holo_platform::jni_util::with_activity;
use holo_platform::render::Renderer;
use holo_platform::waker;
use jni::objects::{JByteArray, JValue};
use jni::{jni_sig, jni_str};

/// Pixels per side. 255 + the atlas's 1 px gap = 256, so a 2048×3072 atlas
/// holds 8 × 12 = 96.
pub const SIZE: u32 = 255;
const WORKERS: usize = 2;
/// Finished thumbnails waiting for the main loop; more waits in the queue.
const READY_MAX: usize = 24;

pub fn key(id: i64) -> String {
    format!("t{id}")
}

#[derive(Default)]
struct State {
    queue: ThumbQueue,
    /// What the loaders need to know about each wanted item: video?, orientation.
    info: HashMap<i64, (bool, u16)>,
    ready: VecDeque<(i64, Vec<u8>)>,
    app: Option<AndroidApp>,
}

pub struct Thumbs {
    shared: Arc<(Mutex<State>, Condvar)>,
}

impl Thumbs {
    pub fn new(app: &AndroidApp) -> Thumbs {
        let shared: Arc<(Mutex<State>, Condvar)> = Arc::default();
        shared.0.lock().unwrap().app = Some(app.clone());
        for n in 0..WORKERS {
            let s = shared.clone();
            thread::Builder::new().name(format!("thumbs{n}")).spawn(move || work(s)).expect("thumb thread");
        }
        Thumbs { shared }
    }

    /// A rebuilt activity: load through the new one.
    pub fn set_app(&self, app: &AndroidApp) {
        self.shared.0.lock().unwrap().app = Some(app.clone());
    }

    /// The thumbnails missing now: on screen first, then just off it.
    /// Each entry: id, video?, orientation.
    pub fn request(&self, missing: &[(i64, bool, u16)]) {
        let (lock, cv) = &*self.shared;
        let mut s = lock.lock().unwrap();
        let ids: Vec<i64> = missing.iter().map(|m| m.0).collect();
        s.queue.want(&ids);
        s.info.clear();
        for &(id, video, orientation) in missing {
            s.info.insert(id, (video, orientation));
        }
        drop(s);
        cv.notify_all();
    }

    pub fn failed(&self, id: i64) -> bool {
        self.shared.0.lock().unwrap().queue.failed(id)
    }

    pub fn clear_failures(&self) {
        self.shared.0.lock().unwrap().queue.clear_failures();
    }

    /// Put finished thumbnails into the renderer, for at most `budget`.
    /// Returns whether more are waiting.
    pub fn upload(&self, gl: &glow::Context, r: &mut Renderer, budget: Duration) -> bool {
        let t = Instant::now();
        loop {
            let next = self.shared.0.lock().unwrap().ready.pop_front();
            let Some((id, rgba)) = next else { return false };
            r.icon(gl, &key(id), &rgba, SIZE, SIZE);
            if t.elapsed() >= budget {
                let more = !self.shared.0.lock().unwrap().ready.is_empty();
                // Room in the waiting list again: let the loaders go on.
                self.shared.1.notify_all();
                return more;
            }
        }
    }

    /// Let the graphics driver finish the uploads, so it can reuse its
    /// copies instead of keeping a pile of them (the launcher's Adreno trap).
    pub fn settle(gl: &glow::Context) {
        use glow::HasContext;
        unsafe { gl.finish() };
    }
}

fn work(shared: Arc<(Mutex<State>, Condvar)>) {
    let (lock, cv) = &*shared;
    loop {
        let (id, video, orientation, app) = {
            let mut s = lock.lock().unwrap();
            loop {
                if s.ready.len() < READY_MAX && s.app.is_some() {
                    if let Some(id) = s.queue.next() {
                        let (video, orientation) = s.info.get(&id).copied().unwrap_or((false, 0));
                        break (id, video, orientation, s.app.clone().unwrap());
                    }
                }
                s = cv.wait(s).unwrap();
            }
        };
        let t = Instant::now();
        let pixels = with_activity(&app, |env, activity| {
            let array = env
                .call_method(
                    activity,
                    jni_str!("loadThumb"),
                    jni_sig!("(JZII)[B"),
                    &[JValue::Long(id), JValue::Bool(video), JValue::Int(orientation as i32), JValue::Int(SIZE as i32)],
                )?
                .l()?;
            if array.is_null() {
                return Ok(None);
            }
            let array = env.cast_local::<JByteArray>(array)?;
            env.convert_byte_array(&array).map(Some)
        });
        let ok = matches!(pixels, Ok(Some(ref p)) if p.len() == (SIZE * SIZE * 4) as usize);
        if t.elapsed() > Duration::from_millis(100) {
            log::info!("slow thumbnail {id}: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        }
        let mut s = lock.lock().unwrap();
        s.queue.done(id, ok);
        if ok && let Ok(Some(p)) = pixels {
            s.ready.push_back((id, p));
        }
        drop(s);
        waker::wake();
    }
}
