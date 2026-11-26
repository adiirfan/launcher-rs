//! The phone's photos and videos, read on a thread of their own.
//!
//! The list from the last run is shown first (saved as a snapshot), so the
//! grid appears at once; then Android is asked afresh, and again whenever
//! its media list changes (a new photo, a download, a delete elsewhere).

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use android_activity::AndroidApp;
use gallery_core::access::{access_from, Access};
use gallery_core::media::{parse_records, MediaItem};
use holo_platform::jni_util::with_activity;
use holo_platform::waker;
use jni::objects::{JByteArray, JClass, JValue};
use jni::sys::{jboolean, jint};
use jni::{jni_sig, jni_str, EnvUnowned};

/// Changes closer together than this are read once.
const SETTLE: Duration = Duration::from_millis(500);

struct Shared {
    app: Option<AndroidApp>,
    dir: Option<PathBuf>,
    access: Access,
    /// Java has answered at least once.
    known: bool,
    /// Android's media list changed (or we just started).
    dirty: bool,
    /// The newest list, not yet taken by the main loop.
    fresh: Option<Arc<Vec<MediaItem>>>,
}

static SHARED: Mutex<Shared> =
    Mutex::new(Shared { app: None, dir: None, access: Access::None, known: false, dirty: true, fresh: None });
static WORK: Condvar = Condvar::new();
static STARTED: OnceLock<()> = OnceLock::new();
static ASKED: Mutex<bool> = Mutex::new(false);

/// Call at the start of every main loop: hands over the current activity
/// and starts the thread the first time.
pub fn start(app: &AndroidApp, dir: PathBuf) {
    {
        let mut s = SHARED.lock().unwrap();
        s.app = Some(app.clone());
        s.dir = Some(dir);
    }
    STARTED.get_or_init(|| {
        thread::Builder::new().name("library".into()).spawn(run).expect("library thread");
    });
    WORK.notify_one();
}

/// The newest list, once per change.
pub fn take_update() -> Option<Arc<Vec<MediaItem>>> {
    SHARED.lock().unwrap().fresh.take()
}

pub fn access() -> Access {
    SHARED.lock().unwrap().access
}

/// Whether Java has reported the permissions yet.
pub fn access_known() -> bool {
    SHARED.lock().unwrap().known
}

/// The permission question was answered (or dismissed) since the last call.
pub fn take_asked() -> bool {
    std::mem::take(&mut *ASKED.lock().unwrap())
}

pub fn request_access(app: &AndroidApp) {
    call(app, jni_str!("requestAccess"));
}

pub fn open_settings(app: &AndroidApp) {
    call(app, jni_str!("openAppSettings"));
}

/// Whether Android would still show its question (false once refused twice).
pub fn can_ask(app: &AndroidApp) -> bool {
    with_activity(app, |env, activity| {
        env.call_method(activity, jni_str!("canAskAccess"), jni_sig!("()Z"), &[])?.z()
    })
    .unwrap_or(true)
}

fn call(app: &AndroidApp, method: &'static jni::strings::JNIStr) {
    let r = with_activity(app, |env, activity| {
        env.call_method(activity, method, jni_sig!("()V"), &[])?;
        Ok(())
    });
    if let Err(e) = r {
        log::warn!("library: {e}");
    }
}

fn snapshot_path(dir: &PathBuf) -> PathBuf {
    dir.join("library.bin")
}

fn run() {
    // First the list from last time, so the grid can show straight away.
    // Nothing, not even last time's list, is shown without permission
    // (it may have been taken away since).
    let dir = loop {
        let s = SHARED.lock().unwrap();
        if let Some(d) = s.dir.clone().filter(|_| s.known && s.access != Access::None) {
            break d;
        }
        drop(WORK.wait(s).unwrap());
    };
    let t = Instant::now();
    if let Ok(bytes) = std::fs::read(snapshot_path(&dir))
        && let Ok(items) = parse_records(&bytes)
    {
        log::info!("library: snapshot {} items in {:.0} ms", items.len(), t.elapsed().as_secs_f64() * 1e3);
        SHARED.lock().unwrap().fresh = Some(Arc::new(items));
        waker::wake();
    }
    loop {
        let app = {
            let mut s = SHARED.lock().unwrap();
            while !(s.dirty && s.access != Access::None && s.app.is_some()) {
                s = WORK.wait(s).unwrap();
            }
            s.dirty = false;
            s.app.clone().unwrap()
        };
        let t = Instant::now();
        let bytes = with_activity(&app, |env, activity| {
            let array = env
                .call_method(activity, jni_str!("queryLibrary"), jni_sig!("(Z)[B"), &[JValue::Bool(false)])?
                .l()?;
            let array = env.cast_local::<JByteArray>(array)?;
            env.convert_byte_array(&array)
        });
        match bytes.map(|b| (parse_records(&b), b)) {
            Ok((Ok(items), raw)) => {
                log::info!("library: {} items in {:.0} ms", items.len(), t.elapsed().as_secs_f64() * 1e3);
                SHARED.lock().unwrap().fresh = Some(Arc::new(items));
                waker::wake();
                save(&dir, &raw);
            }
            Ok((Err(e), _)) => log::warn!("library: unreadable list {e:?}"),
            Err(e) => log::warn!("library: query failed {e}"),
        }
        thread::sleep(SETTLE);
    }
}

/// Written beside and then renamed, so a half-written file is never read.
fn save(dir: &PathBuf, raw: &[u8]) {
    let tmp = dir.join("library.tmp");
    if std::fs::write(&tmp, raw).is_ok() {
        let _ = std::fs::rename(&tmp, snapshot_path(dir));
    }
}

fn mark_dirty() {
    SHARED.lock().unwrap().dirty = true;
    WORK.notify_one();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onLibraryChanged<'l>(_env: EnvUnowned<'l>, _class: JClass<'l>) {
    mark_dirty();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onAsked<'l>(_env: EnvUnowned<'l>, _class: JClass<'l>) {
    *ASKED.lock().unwrap() = true;
    waker::wake();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onAccess<'l>(
    _env: EnvUnowned<'l>,
    _class: JClass<'l>,
    sdk: jint,
    images: jboolean,
    video: jboolean,
    selected: jboolean,
    legacy: jboolean,
) {
    let a = access_from(sdk as u32, images, video, selected, legacy);
    {
        let mut s = SHARED.lock().unwrap();
        if s.access != a {
            log::info!("library: access {a:?}");
            s.dirty = true;
        }
        s.access = a;
        s.known = true;
    }
    WORK.notify_one();
    waker::wake();
}
