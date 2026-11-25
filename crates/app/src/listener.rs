//! What Android's notification listener tells us: which apps have
//! notifications (for dots), and what music is playing (for the controls).
//! Values can arrive before the main loop runs; they wait here until taken.

use std::collections::HashSet;
use std::sync::Mutex;

use android_activity::AndroidApp;
use jni::objects::{JClass, JString, JValue};
use jni::sys::jint;
use jni::{EnvUnowned, jni_sig, jni_str};

use crate::jni_util::with_activity;
use crate::keyboard;

#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaButton {
    Prev = 0,
    PlayPause = 1,
    Next = 2,
    Open = 3,
}

static DOTS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static MEDIA: Mutex<Option<Option<Media>>> = Mutex::new(None);

/// Packages with notifications, when that changed since the last call.
pub fn take_dots() -> Option<HashSet<String>> {
    DOTS.lock().unwrap().take()
}

/// The music now showing (None: nothing playing or paused), when it changed.
pub fn take_media() -> Option<Option<Media>> {
    MEDIA.lock().unwrap().take()
}

pub fn has_access(app: &AndroidApp) -> bool {
    with_activity(app, |env, activity| env.call_method(activity, jni_str!("hasListenerAccess"), jni_sig!("()Z"), &[])?.z()).unwrap_or(false)
}

fn call(app: &AndroidApp, method: &jni::strings::JNIStr, name: &str) {
    if let Err(e) = with_activity(app, |env, activity| {
        env.call_method(activity, method, jni_sig!("()V"), &[])?;
        Ok(())
    }) {
        log::warn!("{name} failed: {e}");
    }
}

/// Android's page where the owner allows notification access.
pub fn open_access_settings(app: &AndroidApp) {
    call(app, jni_str!("openListenerSettings"), "openListenerSettings");
}

/// Start following music sessions (does nothing without access).
pub fn watch_media(app: &AndroidApp) {
    call(app, jni_str!("watchMedia"), "watchMedia");
}

pub fn media_button(app: &AndroidApp, b: MediaButton) {
    if let Err(e) = with_activity(app, |env, activity| {
        env.call_method(activity, jni_str!("mediaButton"), jni_sig!("(I)V"), &[JValue::Int(b as i32)])?;
        Ok(())
    }) {
        log::warn!("mediaButton failed: {e}");
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_app_DotsListener_onDots<'l>(mut unowned_env: EnvUnowned<'l>, _class: JClass<'l>, packages: JString<'l>) {
    let outcome = unowned_env.with_env(|env| -> jni::errors::Result<()> {
        let text: String = packages.mutf8_chars(env)?.into();
        let set: HashSet<String> = text.lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
        log::info!("dots: {} app(s)", set.len());
        *DOTS.lock().unwrap() = Some(set);
        Ok(())
    });
    outcome.resolve::<jni::errors::LogErrorAndDefault>();
    keyboard::wake();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_app_MainActivity_onMedia<'l>(
    mut unowned_env: EnvUnowned<'l>,
    _class: JClass<'l>,
    title: JString<'l>,
    artist: JString<'l>,
    state: jint,
) {
    let outcome = unowned_env.with_env(|env| -> jni::errors::Result<()> {
        let title: String = title.mutf8_chars(env)?.into();
        let artist: String = artist.mutf8_chars(env)?.into();
        // 1 playing, 2 paused; anything else hides the controls.
        let media = (matches!(state, 1 | 2) && !title.is_empty()).then(|| Media { title, artist, playing: state == 1 });
        log::info!("music: {}", match &media {
            Some(m) if m.playing => "playing",
            Some(_) => "paused",
            None => "none",
        });
        *MEDIA.lock().unwrap() = Some(media);
        Ok(())
    });
    outcome.resolve::<jni::errors::LogErrorAndDefault>();
    keyboard::wake();
}
