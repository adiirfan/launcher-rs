//! "Check for updates" in settings: the Java side (Updater.java) asks GitHub
//! and installs; this keeps what it last reported, for the settings row.

use std::sync::Mutex;

use android_activity::AndroidApp;
use jni::objects::{JClass, JString};
use jni::sys::jint;
use jni::{EnvUnowned, jni_sig, jni_str};

use crate::jni_util::with_activity;
use crate::keyboard;

const IDLE: i32 = 0;
const CHECKING: i32 = 1;
const UP_TO_DATE: i32 = 2;
const AVAILABLE: i32 = 3;
const DOWNLOADING: i32 = 4;
const INSTALLING: i32 = 5;
const FAILED: i32 = 6;
const NEEDS_PERMISSION: i32 = 7;

struct Status {
    state: i32,
    text: String,
    percent: i32,
    /// This app's own version, as first reported.
    mine: String,
    /// Bumped on every change, so the settings page lays itself out again.
    generation: u32,
    changed: bool,
}

static STATUS: Mutex<Status> =
    Mutex::new(Status { state: IDLE, text: String::new(), percent: 0, mine: String::new(), generation: 0, changed: false });

/// What the row says under "Check for updates".
pub fn line() -> String {
    let s = STATUS.lock().unwrap();
    match s.state {
        CHECKING => "Checking GitHub…".into(),
        UP_TO_DATE => format!("Version {} is the newest", s.mine),
        AVAILABLE => format!("Version {} is ready. Tap to download and install", s.text),
        DOWNLOADING => format!("Downloading… {}%", s.percent),
        INSTALLING => "Confirm the update in Android's window".into(),
        FAILED => format!("{}. Tap to try again", s.text),
        NEEDS_PERMISSION => "Allow \"Install unknown apps\" for the launcher, then tap again".into(),
        _ if s.mine.is_empty() => "Tap to check for a newer version".into(),
        _ => format!("Version {}. Tap to check for a newer one", s.mine),
    }
}

pub fn generation() -> u32 {
    STATUS.lock().unwrap().generation
}

/// True once after each change: the screen should be drawn again.
pub fn take_changed() -> bool {
    std::mem::take(&mut STATUS.lock().unwrap().changed)
}

/// The row was tapped: check, or install what a check found.
pub fn tapped(app: &AndroidApp) {
    let state = STATUS.lock().unwrap().state;
    let method = match state {
        AVAILABLE | NEEDS_PERMISSION => jni_str!("installUpdate"),
        CHECKING | DOWNLOADING | INSTALLING => return,
        _ => jni_str!("checkForUpdate"),
    };
    let r = with_activity(app, |env, activity| {
        env.call_method(activity, method, jni_sig!("()V"), &[])?;
        Ok(())
    });
    if let Err(e) = r {
        log::warn!("update: {e}");
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_app_Updater_onUpdate<'l>(
    mut unowned_env: EnvUnowned<'l>,
    _class: JClass<'l>,
    state: jint,
    text: JString<'l>,
    percent: jint,
) {
    let outcome = unowned_env.with_env(|env| -> jni::errors::Result<()> {
        let text: String = text.mutf8_chars(env)?.into();
        let mut s = STATUS.lock().unwrap();
        if state == IDLE {
            // Sent when the activity starts: it carries our version.
            s.mine = text;
            if matches!(s.state, CHECKING | DOWNLOADING | INSTALLING) {
                return Ok(());
            }
            s.state = IDLE;
        } else {
            if state == UP_TO_DATE {
                s.mine = text.clone();
            }
            s.state = state;
            s.text = text;
            s.percent = percent;
        }
        s.generation = s.generation.wrapping_add(1);
        s.changed = true;
        log::info!("update: state {state}, {percent}%");
        Ok(())
    });
    outcome.resolve::<jni::errors::LogErrorAndDefault>();
    keyboard::wake();
}
