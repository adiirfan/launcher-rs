//! Small things borrowed from Android: today's date in the phone's own
//! style, and pulling down the notification shade.

use android_activity::AndroidApp;
use jni::objects::{JString, JValue};
use jni::{jni_sig, jni_str};

use crate::jni_util::with_activity;

/// "Wed, Nov 25" (or the phone language's equivalent).
pub fn date_text(app: &AndroidApp) -> Option<String> {
    let result = with_activity(app, |env, _activity| {
        let locale = env
            .call_static_method(jni_str!("java/util/Locale"), jni_str!("getDefault"), jni_sig!("()Ljava/util/Locale;"), &[])?
            .l()?;
        let skeleton = env.new_string("EEEMMMd")?;
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
        let text = env.cast_local::<JString>(text)?;
        Ok(String::from(text.mutf8_chars(env)?))
    });
    result.map_err(|e| log::warn!("date: {e}")).ok()
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
