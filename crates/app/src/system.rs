//! Small things borrowed from Android: today's date in the phone's own
//! style, and pulling down the notification shade.

use android_activity::AndroidApp;
use jni::objects::{JString, JValue};
use jni::{jni_sig, jni_str};

use crate::jni_util::with_activity;

/// The Holo clock's parts: hours ("10" or "22"), minutes (":42"), and the
/// date in capitals ("WEDNESDAY, NOVEMBER 25", in the phone's language).
/// Follows the phone's 12/24-hour setting; 12-hour shows no AM/PM.
pub fn clock_text(app: &AndroidApp) -> Option<(String, String, String)> {
    const HOUR: i32 = 10;
    const HOUR_OF_DAY: i32 = 11;
    const MINUTE: i32 = 12;
    let result = with_activity(app, |env, activity| {
        let cal = env
            .call_static_method(jni_str!("java/util/Calendar"), jni_str!("getInstance"), jni_sig!("()Ljava/util/Calendar;"), &[])?
            .l()?;
        let get = |env: &mut jni::Env, field: i32| -> jni::errors::Result<i32> {
            env.call_method(&cal, jni_str!("get"), jni_sig!("(I)I"), &[JValue::Int(field)])?.i()
        };
        let is24 = env
            .call_static_method(
                jni_str!("android/text/format/DateFormat"),
                jni_str!("is24HourFormat"),
                jni_sig!("(Landroid/content/Context;)Z"),
                &[JValue::Object(activity)],
            )?
            .z()?;
        let hours = if is24 {
            format!("{:02}", get(env, HOUR_OF_DAY)?)
        } else {
            let h = get(env, HOUR)?;
            (if h == 0 { 12 } else { h }).to_string()
        };
        let minutes = format!(":{:02}", get(env, MINUTE)?);

        let locale = env
            .call_static_method(jni_str!("java/util/Locale"), jni_str!("getDefault"), jni_sig!("()Ljava/util/Locale;"), &[])?
            .l()?;
        let skeleton = env.new_string("EEEEMMMMd")?;
        let pattern = env
            .call_static_method(
                jni_str!("android/text/format/DateFormat"),
                jni_str!("getBestDateTimePattern"),
                jni_sig!("(Ljava/util/Locale;Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(&locale), JValue::Object(&skeleton)],
            )?
            .l()?;
        let format = env.new_object(
            jni_str!("java/text/SimpleDateFormat"),
            jni_sig!("(Ljava/lang/String;Ljava/util/Locale;)V"),
            &[JValue::Object(&pattern), JValue::Object(&locale)],
        )?;
        let date = env.new_object(jni_str!("java/util/Date"), jni_sig!("()V"), &[])?;
        let text = env
            .call_method(&format, jni_str!("format"), jni_sig!("(Ljava/util/Date;)Ljava/lang/String;"), &[JValue::Object(&date)])?
            .l()?;
        let upper = env
            .call_method(&text, jni_str!("toUpperCase"), jni_sig!("(Ljava/util/Locale;)Ljava/lang/String;"), &[JValue::Object(&locale)])?
            .l()?;
        let upper = env.cast_local::<JString>(upper)?;
        Ok((hours, minutes, String::from(upper.mutf8_chars(env)?)))
    });
    result.map_err(|e| log::warn!("clock: {e}")).ok()
}

/// Pull down the notification shade, as a swipe down on Home does on Pixels.
pub fn expand_notifications(app: &AndroidApp) {
    let result = with_activity(app, |env, activity| {
        let name = env.new_string("statusbar")?;
        let sbm = env
            .call_method(activity, jni_str!("getSystemService"), jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"), &[JValue::Object(&name)])?
            .l()?;
        if sbm.is_null() {
            return Ok(false);
        }
        env.call_method(&sbm, jni_str!("expandNotificationsPanel"), jni_sig!("()V"), &[])?;
        Ok(true)
    });
    match result {
        Ok(true) => log::info!("notifications opened"),
        Ok(false) => log::warn!("notifications: no status bar service"),
        Err(e) => log::warn!("notifications: {e}"),
    }
}

/// Start an activity for `action` (with optional data URI). False if
/// nothing on the phone handles it.
pub fn start(app: &AndroidApp, action: &str, data: Option<&str>) -> bool {
    let result = with_activity(app, |env, activity| {
        let action = env.new_string(action)?;
        let intent = env.new_object(jni_str!("android/content/Intent"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&action)])?;
        if let Some(data) = data {
            let s = env.new_string(data)?;
            let uri = env
                .call_static_method(jni_str!("android/net/Uri"), jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&s)])?
                .l()?;
            env.call_method(&intent, jni_str!("setData"), jni_sig!("(Landroid/net/Uri;)Landroid/content/Intent;"), &[JValue::Object(&uri)])?;
        }
        env.call_method(activity, jni_str!("startActivity"), jni_sig!("(Landroid/content/Intent;)V"), &[JValue::Object(&intent)])?;
        Ok(())
    });
    match result {
        Ok(()) => true,
        Err(e) => {
            log::warn!("start {action:?} failed: {e}");
            false
        }
    }
}

/// This app's own page in Settings (to turn a permission back on).
pub fn open_own_app_details(app: &AndroidApp) -> bool {
    start(app, "android.settings.APPLICATION_DETAILS_SETTINGS", Some("package:dev.launcherrs.app"))
}

/// The short buzz when a long-press takes hold.
pub fn buzz(app: &AndroidApp) {
    call_activity(app, jni_str!("buzz"), "buzz");
}

/// Android's list of wallpaper pickers.
pub fn pick_wallpaper(app: &AndroidApp) {
    call_activity(app, jni_str!("pickWallpaper"), "pickWallpaper");
}

fn call_activity(app: &AndroidApp, method: &jni::strings::JNIStr, name: &str) {
    let r = with_activity(app, |env, activity| {
        env.call_method(activity, method, jni_sig!("()V"), &[])?;
        Ok(())
    });
    if let Err(e) = r {
        log::warn!("{name} failed: {e}");
    }
}
