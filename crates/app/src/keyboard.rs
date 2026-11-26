//! The keyboard bridge: Java's input view hands us text; we ask Java to show
//! or hide the keyboard.

use std::sync::{Mutex, OnceLock};

use std::time::{Duration, Instant};

use android_activity::{AndroidApp, AndroidAppWaker};
use jni::objects::{JClass, JString, JValue};
use jni::sys::jint;
use jni::{EnvUnowned, jni_sig, jni_str};
use launcher_core::ime::ImeState;

use crate::jni_util::with_activity;

/// The newest text from the keyboard, and when it came.
static LATEST: Mutex<Option<(ImeState, Instant)>> = Mutex::new(None);
static ACTION: Mutex<bool> = Mutex::new(false);
static HOME: Mutex<bool> = Mutex::new(false);
static PERMISSION: Mutex<Option<(i32, bool)>> = Mutex::new(None);
static TRIM: Mutex<Option<i32>> = Mutex::new(None);
static WAKER: OnceLock<AndroidAppWaker> = OnceLock::new();

/// Call once at start so keyboard events can wake the main loop.
pub fn init(app: &AndroidApp) {
    let _ = WAKER.set(app.create_waker());
}

pub fn show(app: &AndroidApp, show: bool) {
    let r = with_activity(app, |env, activity| {
        env.call_method(activity, jni_str!("showKeyboard"), jni_sig!("(Z)V"), &[JValue::Bool(show)])?;
        Ok(())
    });
    if let Err(e) = r {
        log::warn!("showKeyboard({show}) failed: {e}");
    }
}

/// Empty the search text (the keyboard's copy too).
pub fn clear_text(app: &AndroidApp) {
    call_void(app, jni_str!("clearText"), "clearText");
}

/// Permission request codes, so an answer can be matched to its question.
pub const CONTACTS: i32 = 1;
pub const CALENDAR: i32 = 2;

/// Ask Android for the contacts permission; the answer arrives via `take_permission`.
pub fn ask_contacts(app: &AndroidApp) {
    ask_permission(app, "android.permission.READ_CONTACTS", CONTACTS);
}

pub fn ask_permission(app: &AndroidApp, permission: &str, code: i32) {
    let r = with_activity(app, |env, activity| {
        let p = env.new_string(permission)?;
        env.call_method(activity, jni_str!("askPermission"), jni_sig!("(Ljava/lang/String;I)V"), &[JValue::Object(&p), JValue::Int(code)])?;
        Ok(())
    });
    if let Err(e) = r {
        log::warn!("askPermission failed: {e}");
    }
}

fn call_void(app: &AndroidApp, method: &jni::strings::JNIStr, name: &str) {
    let r = with_activity(app, |env, activity| {
        env.call_method(activity, method, jni_sig!("()V"), &[])?;
        Ok(())
    });
    if let Err(e) = r {
        log::warn!("{name} failed: {e}");
    }
}

/// Whether Home was pressed while we were already Home, since the last call.
pub fn take_home() -> bool {
    std::mem::take(&mut *HOME.lock().unwrap())
}

/// The highest memory-trim level Android asked for since the last call.
pub fn take_trim() -> Option<i32> {
    TRIM.lock().unwrap().take()
}

/// The answer to the last permission request (its code, and yes or no), once.
pub fn take_permission() -> Option<(i32, bool)> {
    PERMISSION.lock().unwrap().take()
}

/// The newest text state since the last call.
pub fn take() -> Option<ImeState> {
    let (state, came) = LATEST.lock().unwrap().take()?;
    let waited = came.elapsed();
    if waited > Duration::from_millis(100) {
        log::info!("slow text: waited {:.0} ms to be shown", waited.as_secs_f64() * 1e3);
    }
    Some(state)
}

/// Whether Enter / Search was pressed since the last call.
pub fn take_action() -> bool {
    std::mem::take(&mut *ACTION.lock().unwrap())
}

pub(crate) fn wake() {
    if let Some(w) = WAKER.get() {
        w.wake();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_app_MainActivity_onText<'l>(
    mut unowned_env: EnvUnowned<'l>,
    _class: JClass<'l>,
    text: JString<'l>,
    sel_start: jint,
    sel_end: jint,
    comp_start: jint,
    comp_end: jint,
) {
    let outcome = unowned_env.with_env(|env| -> jni::errors::Result<()> {
        let text: String = text.mutf8_chars(env)?.into();
        let state = ImeState::from_java(&text, sel_start, sel_end, comp_start, comp_end);
        *LATEST.lock().unwrap() = Some((state, Instant::now()));
        Ok(())
    });
    outcome.resolve::<jni::errors::LogErrorAndDefault>();
    wake();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_app_MainActivity_onAction<'l>(
    _unowned_env: EnvUnowned<'l>,
    _class: JClass<'l>,
) {
    *ACTION.lock().unwrap() = true;
    wake();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_app_MainActivity_onTrim<'l>(_env: EnvUnowned<'l>, _class: JClass<'l>, level: jint) {
    let mut t = TRIM.lock().unwrap();
    *t = Some(t.map_or(level, |old| old.max(level)));
    drop(t);
    wake();
}

/// System bar sizes in pixels (top, bottom, left, right), as Android last reported them.
static INSETS: Mutex<Option<[i32; 4]>> = Mutex::new(None);

pub fn bar_insets() -> Option<[i32; 4]> {
    *INSETS.lock().unwrap()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_app_MainActivity_onInsets<'l>(
    _env: EnvUnowned<'l>,
    _class: JClass<'l>,
    top: jint,
    bottom: jint,
    left: jint,
    right: jint,
) {
    *INSETS.lock().unwrap() = Some([top, bottom, left, right]);
    wake();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_app_MainActivity_onHome<'l>(_env: EnvUnowned<'l>, _class: JClass<'l>) {
    *HOME.lock().unwrap() = true;
    wake();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_app_MainActivity_onPermission<'l>(
    _env: EnvUnowned<'l>,
    _class: JClass<'l>,
    code: jint,
    granted: jni::sys::jboolean,
) {
    *PERMISSION.lock().unwrap() = Some((code, granted));
    wake();
}
