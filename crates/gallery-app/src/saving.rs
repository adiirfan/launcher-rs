//! New files for edited and converted copies (Saver.java). Safe to call
//! from worker threads.

use android_activity::AndroidApp;
use gallery_core::copies::{copy_folder, copy_name};
use holo_platform::jni_util::with_activity;
use jni::objects::{JObject, JString, JValue};
use jni::{jni_sig, jni_str, Env};

fn text(env: &mut Env, obj: JObject) -> jni::errors::Result<Option<String>> {
    if obj.is_null() {
        return Ok(None);
    }
    let s = env.cast_local::<JString>(obj)?;
    Ok(Some(s.mutf8_chars(env)?.into()))
}

fn warn<T>(what: &str, r: jni::errors::Result<T>) -> Option<T> {
    r.map_err(|e| log::warn!("{what} failed: {e}")).ok()
}

/// The folder an item is in, relative to the storage root ("" if unknown).
pub fn rel_path(app: &AndroidApp, id: i64, video: bool) -> String {
    warn(
        "relPath",
        with_activity(app, |env, a| {
            let o = env.call_method(a, jni_str!("saverRelPath"), jni_sig!("(JZ)Ljava/lang/String;"), &[JValue::Long(id), JValue::Bool(video)])?.l()?;
            text(env, o)
        }),
    )
    .flatten()
    .unwrap_or_default()
}

fn exists(app: &AndroidApp, rel: &str, name: &str) -> bool {
    warn(
        "exists",
        with_activity(app, |env, a| {
            let (r, n) = (env.new_string(rel)?, env.new_string(name)?);
            env.call_method(a, jni_str!("saverExists"), jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Z"), &[JValue::Object(&r), JValue::Object(&n)])?.z()
        }),
    )
    .unwrap_or(false)
}

/// A copy being written. Call `finish` when the bytes are in, or drop it
/// (or call `abandon`) to throw it away.
pub struct Copy {
    app: AndroidApp,
    pub uri: String,
    pub name: String,
    pub folder: String,
    taken_ms: i64,
    done: bool,
}

/// Where and under what name a copy of `original` (an item in the library)
/// goes. `suffix` like "_crop", `ext` like "jpg".
pub fn begin(app: &AndroidApp, id: i64, original: &str, video_item: bool, suffix: &str, ext: &str, mime: &str, taken_ms: i64, save_as_video: bool) -> Option<Copy> {
    let sdk = crate::actions::sdk(app);
    let folder = copy_folder(&rel_path(app, id, video_item), save_as_video, sdk);
    let name = copy_name(original, suffix, ext, &|n| exists(app, &folder, n));
    let uri = warn(
        "begin",
        with_activity(app, |env, a| {
            let (r, n, m) = (env.new_string(&folder)?, env.new_string(&name)?, env.new_string(mime)?);
            let o = env
                .call_method(
                    a,
                    jni_str!("saverBegin"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JZ)Ljava/lang/String;"),
                    &[JValue::Object(&r), JValue::Object(&n), JValue::Object(&m), JValue::Long(taken_ms), JValue::Bool(save_as_video)],
                )?
                .l()?;
            text(env, o)
        }),
    )
    .flatten()?;
    log::info!("saving copy {folder}{name}");
    Some(Copy { app: app.clone(), uri, name, folder, taken_ms, done: false })
}

impl Copy {
    /// A file descriptor to write to; the caller closes it.
    pub fn open_fd(&self, mode: &str) -> Option<i32> {
        let fd = warn(
            "openFd",
            with_activity(&self.app, |env, a| {
                let (u, m) = (env.new_string(&self.uri)?, env.new_string(mode)?);
                env.call_method(a, jni_str!("saverOpenFd"), jni_sig!("(Ljava/lang/String;Ljava/lang/String;)I"), &[JValue::Object(&u), JValue::Object(&m)])?.i()
            }),
        )?;
        (fd >= 0).then_some(fd)
    }

    pub fn finish(mut self) -> bool {
        self.done = true;
        warn(
            "finish",
            with_activity(&self.app, |env, a| {
                let u = env.new_string(&self.uri)?;
                env.call_method(a, jni_str!("saverFinish"), jni_sig!("(Ljava/lang/String;J)Z"), &[JValue::Object(&u), JValue::Long(self.taken_ms)])?.z()
            }),
        )
        .unwrap_or(false)
    }

    pub fn abandon(mut self) {
        self.throw_away();
    }

    fn throw_away(&mut self) {
        if std::mem::replace(&mut self.done, true) {
            return;
        }
        let _ = warn(
            "abandon",
            with_activity(&self.app, |env, a| {
                let u = env.new_string(&self.uri)?;
                env.call_method(a, jni_str!("saverAbandon"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&u)]).map(|_| ())
            }),
        );
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        self.throw_away();
    }
}
