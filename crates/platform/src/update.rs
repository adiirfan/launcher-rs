//! "Check for updates" in settings, for the launcher and the gallery: the
//! Java side (each app's Updater.java) asks GitHub and installs; this keeps
//! what it last reported, for the settings row. Each app exports the JNI
//! entry for its own Updater class with `updater_jni!`.

use std::sync::Mutex;

use android_activity::AndroidApp;
use jni::objects::JString;
use jni::{jni_sig, jni_str};

use crate::jni_util::with_activity;

#[doc(hidden)]
pub use jni;

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

/// What the row says under "Check for updates". `who` names the app in
/// the "Install unknown apps" hint.
pub fn line(who: &str) -> String {
    let s = STATUS.lock().unwrap();
    match s.state {
        CHECKING => "Checking GitHub…".into(),
        UP_TO_DATE => format!("Version {} is the newest", s.mine),
        AVAILABLE => format!("Version {} is ready. Tap to download and install", s.text),
        DOWNLOADING => format!("Downloading… {}%", s.percent),
        INSTALLING => "Confirm the update in Android's window".into(),
        FAILED => format!("{}. Tap to try again", s.text),
        NEEDS_PERMISSION => format!("Allow \"Install unknown apps\" for {who}, then tap again"),
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

/// What Java reported; called from the app's `updater_jni!` entry.
pub fn on_update(env: &mut jni::Env, state: i32, text: &JString, percent: i32) -> jni::errors::Result<()> {
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
}

/// Export `Updater.onUpdate` for one app's Java class, waking its loop with
/// `$wake` after each report:
/// `updater_jni!(Java_dev_launcherrs_app_Updater_onUpdate, crate::keyboard::wake);`
#[macro_export]
macro_rules! updater_jni {
    ($fn_name:ident, $wake:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn $fn_name<'l>(
            mut unowned_env: $crate::update::jni::EnvUnowned<'l>,
            _class: $crate::update::jni::objects::JClass<'l>,
            state: $crate::update::jni::sys::jint,
            text: $crate::update::jni::objects::JString<'l>,
            percent: $crate::update::jni::sys::jint,
        ) {
            let outcome = unowned_env.with_env(|env| $crate::update::on_update(env, state, &text, percent));
            outcome.resolve::<$crate::update::jni::errors::LogErrorAndDefault>();
            $wake();
        }
    };
}
