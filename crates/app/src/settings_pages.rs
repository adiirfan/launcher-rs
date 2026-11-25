//! Which of Android's settings pages exist on this phone, and opening them.

use std::collections::HashSet;

use android_activity::AndroidApp;
use jni::objects::JValue;
use jni::{jni_sig, jni_str};
use launcher_core::search::SETTINGS_PAGES;

use crate::jni_util::with_activity;

/// Actions from the settings table that some app on this phone can open.
/// Some makers leave pages out, so those never appear in results.
pub fn available(app: &AndroidApp) -> HashSet<&'static str> {
    const MATCH_DEFAULT_ONLY: i32 = 0x0001_0000;
    let result = with_activity(app, |env, activity| {
        let pm = env
            .call_method(activity, jni_str!("getPackageManager"), jni_sig!("()Landroid/content/pm/PackageManager;"), &[])?
            .l()?;
        let mut out = HashSet::new();
        for page in SETTINGS_PAGES {
            let found = env.with_local_frame(8, |env| -> jni::errors::Result<bool> {
                let action = env.new_string(page.action)?;
                let intent = env.new_object(jni_str!("android/content/Intent"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&action)])?;
                let info = env
                    .call_method(
                        &pm,
                        jni_str!("resolveActivity"),
                        jni_sig!("(Landroid/content/Intent;I)Landroid/content/pm/ResolveInfo;"),
                        &[JValue::Object(&intent), JValue::Int(MATCH_DEFAULT_ONLY)],
                    )?
                    .l()?;
                Ok(!info.is_null())
            })?;
            if found {
                out.insert(page.action);
            }
        }
        Ok(out)
    });
    let set = result.unwrap_or_else(|e| {
        log::warn!("settings pages: {e}");
        HashSet::new()
    });
    log::info!("settings pages on this phone: {} of {}", set.len(), SETTINGS_PAGES.len());
    set
}
