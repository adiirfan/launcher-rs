//! The next calendar event, for the line under the date.

use android_activity::AndroidApp;
use jni::objects::{JString, JValue};
use jni::{jni_sig, jni_str};

use crate::jni_util::with_activity;
use crate::keyboard;

const PERMISSION: &str = "android.permission.READ_CALENDAR";

pub fn has_permission(app: &AndroidApp) -> bool {
    with_activity(app, |env, activity| {
        let p = env.new_string(PERMISSION)?;
        Ok(env.call_method(activity, jni_str!("checkSelfPermission"), jni_sig!("(Ljava/lang/String;)I"), &[JValue::Object(&p)])?.i()? == 0)
    })
    .unwrap_or(false)
}

pub fn ask(app: &AndroidApp) {
    keyboard::ask_permission(app, PERMISSION, keyboard::CALENDAR);
}

/// When it starts (ms since 1970) and how it reads ("Stand-up · 10:00",
/// "Dentist · tomorrow 9:00", "Stand-up · now"). None when there's none in
/// the next day, or no permission.
pub fn next_event(app: &AndroidApp) -> Option<(i64, String)> {
    let text = with_activity(app, |env, activity| {
        let s = env.call_method(activity, jni_str!("nextEvent"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
        let s = env.cast_local::<JString>(s)?;
        Ok(String::from(s.mutf8_chars(env)?))
    });
    match text {
        Ok(t) => {
            let (begin, line) = t.split_once('\t')?;
            Some((begin.parse().ok()?, line.to_string()))
        }
        Err(e) => {
            log::warn!("next event failed: {e}");
            None
        }
    }
}
