//! The keyboard bridge: Java's input view hands us text; we ask Java to show
//! or hide the keyboard.

use std::sync::{Mutex, OnceLock};

use android_activity::{AndroidApp, AndroidAppWaker};
use jni::objects::{JClass, JString, JValue};
use jni::sys::jint;
use jni::{EnvUnowned, jni_sig, jni_str};
use launcher_core::ime::ImeState;

use crate::jni_util::with_activity;

static LATEST: Mutex<Option<ImeState>> = Mutex::new(None);
static ACTION: Mutex<bool> = Mutex::new(false);
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

/// The newest text state since the last call.
pub fn take() -> Option<ImeState> {
    LATEST.lock().unwrap().take()
}

/// Whether Enter / Search was pressed since the last call.
pub fn take_action() -> bool {
    std::mem::take(&mut *ACTION.lock().unwrap())
}

fn wake() {
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
        *LATEST.lock().unwrap() = Some(state);
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
