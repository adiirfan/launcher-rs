//! Which photos are motion photos. Each is checked once, by a thumbnail
//! worker the first time its thumbnail loads, and the answers are kept in
//! motion.txt so the grid knows straight away next time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use android_activity::AndroidApp;
use gallery_core::motion::{is_motion, parse, to_text, SCAN};
use holo_platform::jni_util::with_activity;
use jni::objects::{JByteArray, JValue};
use jni::{jni_sig, jni_str};

/// Answers not yet written: saved after this many, and when the app goes
/// to the background.
const SAVE_EVERY: usize = 64;

struct Store {
    path: PathBuf,
    known: HashMap<i64, bool>,
    unsaved: usize,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

pub fn load(data_dir: &Path) {
    let path = data_dir.join("motion.txt");
    let known = std::fs::read_to_string(&path).map(|t| parse(&t)).unwrap_or_default();
    *STORE.lock().unwrap() = Some(Store { path, known, unsaved: 0 });
}

/// A motion photo, as far as we know yet.
pub fn is(id: i64) -> bool {
    STORE.lock().unwrap().as_ref().is_some_and(|s| s.known.get(&id) == Some(&true))
}

fn write(s: &mut Store) {
    let tmp = s.path.with_extension("tmp");
    if std::fs::write(&tmp, to_text(&s.known)).is_ok() && std::fs::rename(&tmp, &s.path).is_ok() {
        s.unsaved = 0;
    }
}

pub fn save() {
    if let Some(s) = STORE.lock().unwrap().as_mut()
        && s.unsaved > 0
    {
        write(s);
    }
}

/// Look at the picture `id` if it hasn't been yet. Runs on a worker.
pub fn check(app: &AndroidApp, id: i64) {
    if id < 0 || STORE.lock().unwrap().as_ref().is_none_or(|s| s.known.contains_key(&id)) {
        return;
    }
    let ends = with_activity(app, |env, a| {
        let o = env.call_method(a, jni_str!("motionEnds"), jni_sig!("(JI)[B"), &[JValue::Long(id), JValue::Int(SCAN as i32)])?.l()?;
        if o.is_null() {
            return Ok(None);
        }
        let b = env.cast_local::<JByteArray>(o)?;
        Ok(Some(env.convert_byte_array(&b)?))
    });
    // Unreadable now (no permission yet, file busy): try again next time.
    let Ok(Some(ends)) = ends else { return };
    let motion = is_motion(&ends);
    if motion {
        log::info!("motion photo: {id}");
    }
    let mut g = STORE.lock().unwrap();
    if let Some(s) = g.as_mut() {
        s.known.insert(id, motion);
        s.unsaved += 1;
        if s.unsaved >= SAVE_EVERY {
            write(s);
        }
    }
}
