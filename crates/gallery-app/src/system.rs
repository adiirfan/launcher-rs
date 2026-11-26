//! Asking Android to do things for us through the activity.

use android_activity::AndroidApp;
use holo_platform::jni_util::with_activity;
use jni::{jni_sig, jni_str};

/// Close the app (Back on the first screen).
pub fn finish(app: &AndroidApp) {
    let r = with_activity(app, |env, activity| {
        env.call_method(activity, jni_str!("finish"), jni_sig!("()V"), &[])?;
        Ok(())
    });
    if let Err(e) = r {
        log::warn!("finish failed: {e}");
    }
}
