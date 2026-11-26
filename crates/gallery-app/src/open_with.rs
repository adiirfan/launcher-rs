//! Pictures and videos other apps hand us ("Open with"). Java keeps the
//! link; we get it here and open it in the viewer.

use std::sync::Mutex;

use android_activity::AndroidApp;
use gallery_core::media::MediaItem;
use gallery_core::open_with::kind_of;
use holo_platform::jni_util::with_activity;
use jni::objects::{JClass, JString};
use jni::{jni_sig, jni_str, EnvUnowned};

/// The id Java reads the handed-over link through (see `Thumbs.uri`).
pub const FOREIGN_ID: i64 = i64::MIN;

/// What another app handed over.
pub struct Handed {
    pub uri: String,
    pub mime: String,
    pub name: String,
    pub size: u64,
    pub modified_ms: i64,
}

static PENDING: Mutex<Option<Handed>> = Mutex::new(None);

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onOpen<'l>(
    mut unowned_env: EnvUnowned<'l>,
    _class: JClass<'l>,
    uri: JString<'l>,
    mime: JString<'l>,
    name: JString<'l>,
    size: jni::sys::jlong,
    modified_ms: jni::sys::jlong,
) {
    let outcome = unowned_env.with_env(|env| -> jni::errors::Result<()> {
        let uri: String = uri.mutf8_chars(env)?.into();
        let mime: String = mime.mutf8_chars(env)?.into();
        let name: String = name.mutf8_chars(env)?.into();
        log::info!("open with: {mime}");
        *PENDING.lock().unwrap() = Some(Handed { uri, mime, name, size: size.max(0) as u64, modified_ms });
        Ok(())
    });
    outcome.resolve::<jni::errors::LogErrorAndDefault>();
    holo_platform::waker::wake();
}

/// A link another app handed over.
pub fn take() -> Option<Handed> {
    PENDING.lock().unwrap().take()
}

/// The one-item list for a link outside the library.
pub fn foreign_item(h: &Handed) -> MediaItem {
    MediaItem { id: FOREIGN_ID, kind: kind_of(&h.mime), mime: h.mime.clone(), name: h.name.clone(), size: h.size, added_ms: h.modified_ms, ..MediaItem::default() }
}

/// Back to the app that sent us here, leaving the gallery as it was.
pub fn leave(app: &AndroidApp) {
    if let Err(e) = with_activity(app, |env, a| env.call_method(a, jni_str!("moveTaskToBack"), jni_sig!("(Z)Z"), &[jni::objects::JValue::Bool(true)]).map(|_| ())) {
        log::warn!("leave failed: {e}");
    }
}
