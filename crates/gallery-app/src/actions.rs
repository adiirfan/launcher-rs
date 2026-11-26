//! Sharing, the Bin and favourites, through Java. Android 11+ has its own
//! Bin and asks the owner to confirm; Android 8-10 has none, so deleted
//! files move into our folder and the list lives in `bin.txt` there.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use android_activity::AndroidApp;
use gallery_core::own_bin::{self, BinEntry};
use holo_platform::jni_util::with_activity;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::{jni_sig, jni_str, Env, EnvUnowned};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Op {
    Trash = 1,
    Restore = 2,
    Delete = 3,
    Favourite = 4,
}

static DONE: Mutex<VecDeque<(i32, bool)>> = Mutex::new(VecDeque::new());

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onActionDone<'l>(_env: EnvUnowned<'l>, _class: JClass<'l>, kind: jni::sys::jint, ok: jni::sys::jboolean) {
    DONE.lock().unwrap().push_back((kind, ok));
    holo_platform::waker::wake();
}

/// Answers from Android's confirm dialogs: (which, done?).
pub fn take_done() -> Vec<(Op, bool)> {
    DONE.lock()
        .unwrap()
        .drain(..)
        .filter_map(|(k, ok)| {
            let op = match k {
                1 => Op::Trash,
                2 => Op::Restore,
                3 => Op::Delete,
                4 => Op::Favourite,
                _ => return None,
            };
            Some((op, ok))
        })
        .collect()
}

pub fn has_done() -> bool {
    !DONE.lock().unwrap().is_empty()
}

/// The phone's Android version (API level).
pub fn sdk(app: &AndroidApp) -> u32 {
    static SDK: OnceLock<u32> = OnceLock::new();
    *SDK.get_or_init(|| with_activity(app, |env, a| env.call_method(a, jni_str!("sdk"), jni_sig!("()I"), &[])?.i()).map_or(30, |v| v as u32))
}

/// Android keeps a Bin of its own (11 and later).
pub fn system_bin(app: &AndroidApp) -> bool {
    sdk(app) >= 30
}

fn arrays<'l>(env: &mut Env<'l>, items: &[(i64, bool)]) -> jni::errors::Result<(JObject<'l>, JObject<'l>)> {
    let ids = env.new_long_array(items.len())?;
    ids.set_region(env, 0, &items.iter().map(|i| i.0).collect::<Vec<_>>())?;
    let video = env.new_boolean_array(items.len())?;
    video.set_region(env, 0, &items.iter().map(|i| i.1).collect::<Vec<_>>())?;
    Ok((JObject::from(ids), JObject::from(video)))
}

fn logged<T: Default>(what: &str, r: jni::errors::Result<T>) -> T {
    r.unwrap_or_else(|e| {
        log::warn!("{what} failed: {e}");
        T::default()
    })
}

/// Hand the items to another app. Each: id, video?
pub fn share(app: &AndroidApp, items: &[(i64, bool)]) {
    logged(
        "share",
        with_activity(app, |env, a| {
            let (ids, video) = arrays(env, items)?;
            env.call_method(a, jni_str!("share"), jni_sig!("([J[Z)V"), &[JValue::Object(&ids), JValue::Object(&video)])?;
            Ok(())
        }),
    )
}

/// Android 11+: ask Android to do `op`; the answer comes through `take_done`.
pub fn request(app: &AndroidApp, op: Op, items: &[(i64, bool)], on: bool) {
    logged(
        "media request",
        with_activity(app, |env, a| {
            let (ids, video) = arrays(env, items)?;
            let args = [JValue::Int(op as i32), JValue::Object(&ids), JValue::Object(&video), JValue::Bool(on)];
            env.call_method(a, jni_str!("mediaRequest"), jni_sig!("(I[J[ZZ)V"), &args)?;
            Ok(())
        }),
    )
}

fn string_result(env: &mut Env, obj: JObject) -> jni::errors::Result<String> {
    if obj.is_null() {
        return Ok(String::new());
    }
    let s = env.cast_local::<JString>(obj)?;
    Ok(s.mutf8_chars(env)?.into())
}

/// Our own bin's folder (Android 8-10).
pub fn bin_dir(app: &AndroidApp) -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let s = logged(
            "binDir",
            with_activity(app, |env, a| {
                let o = env.call_method(a, jni_str!("binDir"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
                string_result(env, o)
            }),
        );
        (!s.is_empty()).then(|| PathBuf::from(s))
    })
    .clone()
}

fn list_path(dir: &Path) -> PathBuf {
    dir.join("bin.txt")
}

pub fn own_bin(app: &AndroidApp) -> Vec<BinEntry> {
    bin_dir(app).and_then(|d| std::fs::read_to_string(list_path(&d)).ok()).map(|t| own_bin::from_text(&t)).unwrap_or_default()
}

fn save_own_bin(app: &AndroidApp, entries: &[BinEntry]) {
    if let Some(d) = bin_dir(app) {
        let tmp = d.join("bin.tmp");
        if std::fs::write(&tmp, own_bin::to_text(entries)).is_ok() {
            let _ = std::fs::rename(&tmp, list_path(&d));
        }
    }
}

/// Android 8-10: move the files into our bin and note them.
pub fn trash_legacy(app: &AndroidApp, items: &[(i64, bool)]) -> usize {
    let text = logged(
        "trashLegacy",
        with_activity(app, |env, a| {
            let (ids, video) = arrays(env, items)?;
            let o = env.call_method(a, jni_str!("trashLegacy"), jni_sig!("([J[Z)Ljava/lang/String;"), &[JValue::Object(&ids), JValue::Object(&video)])?.l()?;
            string_result(env, o)
        }),
    );
    let moved = own_bin::from_text(&text);
    if !moved.is_empty() {
        let mut all = own_bin(app);
        all.extend(moved.iter().cloned());
        save_own_bin(app, &all);
    }
    moved.len()
}

/// Android 8-10: put these back where they were.
pub fn restore_legacy(app: &AndroidApp, which: &[BinEntry]) {
    let mut all = own_bin(app);
    for e in which {
        let ok = logged(
            "restoreLegacy",
            with_activity(app, |env, a| {
                let s = env.new_string(&e.stored)?;
                let o = env.new_string(&e.original_path)?;
                env.call_method(a, jni_str!("restoreLegacy"), jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Z"), &[JValue::Object(&s), JValue::Object(&o)])?.z()
            }),
        );
        if ok {
            all.retain(|x| x != e);
        } else {
            log::warn!("could not put back {}", e.original_path);
        }
    }
    save_own_bin(app, &all);
}

/// Android 8-10: gone for good.
pub fn delete_legacy(app: &AndroidApp, which: &[BinEntry]) {
    let mut all = own_bin(app);
    for e in which {
        let _ = std::fs::remove_file(&e.stored);
        all.retain(|x| x != e);
    }
    save_own_bin(app, &all);
}

/// Android 8-10, at start: anything binned over 30 days ago goes for good.
pub fn purge_expired(app: &AndroidApp, now_ms: i64) {
    let all = own_bin(app);
    let (old, keep): (Vec<BinEntry>, Vec<BinEntry>) = all.into_iter().partition(|e| own_bin::expired(e, now_ms));
    if !old.is_empty() {
        for e in &old {
            let _ = std::fs::remove_file(&e.stored);
        }
        log::info!("bin: {} expired items deleted", old.len());
        save_own_bin(app, &keep);
    }
}

/// Media management allowed: Android's confirm dialogs are skipped.
pub fn can_manage_media(app: &AndroidApp) -> bool {
    logged("canManageMedia", with_activity(app, |env, a| env.call_method(a, jni_str!("canManageMedia"), jni_sig!("()Z"), &[])?.z()))
}

/// Android 12+: the page where the owner allows media management.
pub fn open_manage_media(app: &AndroidApp) {
    logged("openManageMedia", with_activity(app, |env, a| env.call_method(a, jni_str!("openManageMedia"), jni_sig!("()V"), &[]).map(|_| ())));
}

/// Android 8-10 favourites: our own list of ids, one per line.
pub struct Favourites {
    path: PathBuf,
    pub ids: HashSet<i64>,
}

impl Favourites {
    pub fn load(data_dir: &Path) -> Favourites {
        let path = data_dir.join("favourites.txt");
        let ids = std::fs::read_to_string(&path).map(|t| t.lines().filter_map(|l| l.trim().parse().ok()).collect()).unwrap_or_default();
        Favourites { path, ids }
    }

    pub fn set(&mut self, id: i64, on: bool) {
        if on {
            self.ids.insert(id);
        } else {
            self.ids.remove(&id);
        }
        let text: String = self.ids.iter().map(|id| format!("{id}\n")).collect();
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}
