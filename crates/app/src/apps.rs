//! Installed apps: list them, load an icon, launch one.

use android_activity::AndroidApp;
use jni::objects::{JObject, JValue};
use jni::{Env, jni_sig, jni_str};

use crate::jni_util::with_activity;

#[derive(Clone, Debug)]
pub struct AppEntry {
    pub label: String,
    pub package: String,
    pub activity: String,
}

/// Local refs made per app; each app gets its own frame so a long list
/// never fills Android's local reference table.
const FRAME: usize = 16;

fn launcher_apps<'l>(env: &mut Env<'l>, activity: &JObject) -> jni::errors::Result<JObject<'l>> {
    let name = env.new_string("launcherapps")?;
    env.call_method(
        activity,
        jni_str!("getSystemService"),
        jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
        &[JValue::Object(&name)],
    )?
    .l()
}

fn my_user<'l>(env: &mut Env<'l>) -> jni::errors::Result<JObject<'l>> {
    env.call_static_method(
        jni_str!("android/os/Process"),
        jni_str!("myUserHandle"),
        jni_sig!("()Landroid/os/UserHandle;"),
        &[],
    )?
    .l()
}

fn java_string(env: &mut Env, obj: &JObject) -> jni::errors::Result<String> {
    if obj.is_null() {
        return Ok(String::new());
    }
    let s = env
        .call_method(obj, jni_str!("toString"), jni_sig!("()Ljava/lang/String;"), &[])?
        .l()?;
    let s = env.cast_local::<jni::objects::JString>(s)?;
    Ok(s.mutf8_chars(env)?.into())
}

/// Every launchable activity for this user, sorted by label (case-insensitive).
pub fn list_apps(app: &AndroidApp) -> Vec<AppEntry> {
    let result = with_activity(app, |env, activity| {
        let la = launcher_apps(env, activity)?;
        let user = my_user(env)?;
        let list = env
            .call_method(
                &la,
                jni_str!("getActivityList"),
                jni_sig!("(Ljava/lang/String;Landroid/os/UserHandle;)Ljava/util/List;"),
                &[JValue::Object(&JObject::null()), JValue::Object(&user)],
            )?
            .l()?;
        let n = env.call_method(&list, jni_str!("size"), jni_sig!("()I"), &[])?.i()?;
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let entry = env.with_local_frame(FRAME, |env| -> jni::errors::Result<AppEntry> {
                let info = env
                    .call_method(&list, jni_str!("get"), jni_sig!("(I)Ljava/lang/Object;"), &[JValue::Int(i)])?
                    .l()?;
                let label = env
                    .call_method(&info, jni_str!("getLabel"), jni_sig!("()Ljava/lang/CharSequence;"), &[])?
                    .l()?;
                let comp = env
                    .call_method(&info, jni_str!("getComponentName"), jni_sig!("()Landroid/content/ComponentName;"), &[])?
                    .l()?;
                let pkg = env
                    .call_method(&comp, jni_str!("getPackageName"), jni_sig!("()Ljava/lang/String;"), &[])?
                    .l()?;
                let cls = env
                    .call_method(&comp, jni_str!("getClassName"), jni_sig!("()Ljava/lang/String;"), &[])?
                    .l()?;
                Ok(AppEntry {
                    label: java_string(env, &label)?,
                    package: java_string(env, &pkg)?,
                    activity: java_string(env, &cls)?,
                })
            })?;
            out.push(entry);
        }
        Ok(out)
    });
    let mut apps = result.unwrap_or_else(|e| {
        log::error!("list_apps failed: {e}");
        Vec::new()
    });
    apps.sort_by_key(|a| a.label.to_lowercase());
    apps
}

fn component<'l>(env: &mut Env<'l>, entry: &AppEntry) -> jni::errors::Result<JObject<'l>> {
    let pkg = env.new_string(&entry.package)?;
    let cls = env.new_string(&entry.activity)?;
    env.new_object(
        jni_str!("android/content/ComponentName"),
        jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
        &[JValue::Object(&pkg), JValue::Object(&cls)],
    )
}

/// The app's icon drawn at `size_px` square, as premultiplied RGBA bytes.
/// `None` if the app is gone or its icon can't be drawn.
pub fn load_icon_rgba(app: &AndroidApp, entry: &AppEntry, size_px: i32) -> Option<(Vec<u8>, i32, i32)> {
    let density = app.config().density().unwrap_or(0) as i32;
    let result = with_activity(app, |env, activity| {
        let la = launcher_apps(env, activity)?;
        let user = my_user(env)?;
        let comp = component(env, entry)?;
        let action = env.new_string("android.intent.action.MAIN")?;
        let intent = env.new_object(
            jni_str!("android/content/Intent"),
            jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::Object(&action)],
        )?;
        env.call_method(
            &intent,
            jni_str!("setComponent"),
            jni_sig!("(Landroid/content/ComponentName;)Landroid/content/Intent;"),
            &[JValue::Object(&comp)],
        )?;
        let info = env
            .call_method(
                &la,
                jni_str!("resolveActivity"),
                jni_sig!("(Landroid/content/Intent;Landroid/os/UserHandle;)Landroid/content/pm/LauncherActivityInfo;"),
                &[JValue::Object(&intent), JValue::Object(&user)],
            )?
            .l()?;
        if info.is_null() {
            return Ok(None);
        }
        let drawable = env
            .call_method(&info, jni_str!("getIcon"), jni_sig!("(I)Landroid/graphics/drawable/Drawable;"), &[JValue::Int(density)])?
            .l()?;
        if drawable.is_null() {
            return Ok(None);
        }
        let argb = env
            .get_static_field(
                jni_str!("android/graphics/Bitmap$Config"),
                jni_str!("ARGB_8888"),
                jni_sig!("Landroid/graphics/Bitmap$Config;"),
            )?
            .l()?;
        let bitmap = env
            .call_static_method(
                jni_str!("android/graphics/Bitmap"),
                jni_str!("createBitmap"),
                jni_sig!("(IILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;"),
                &[JValue::Int(size_px), JValue::Int(size_px), JValue::Object(&argb)],
            )?
            .l()?;
        let canvas = env.new_object(
            jni_str!("android/graphics/Canvas"),
            jni_sig!("(Landroid/graphics/Bitmap;)V"),
            &[JValue::Object(&bitmap)],
        )?;
        env.call_method(
            &drawable,
            jni_str!("setBounds"),
            jni_sig!("(IIII)V"),
            &[JValue::Int(0), JValue::Int(0), JValue::Int(size_px), JValue::Int(size_px)],
        )?;
        env.call_method(&drawable, jni_str!("draw"), jni_sig!("(Landroid/graphics/Canvas;)V"), &[JValue::Object(&canvas)])?;
        let buffer = env
            .call_static_method(
                jni_str!("java/nio/ByteBuffer"),
                jni_str!("allocate"),
                jni_sig!("(I)Ljava/nio/ByteBuffer;"),
                &[JValue::Int(size_px * size_px * 4)],
            )?
            .l()?;
        env.call_method(&bitmap, jni_str!("copyPixelsToBuffer"), jni_sig!("(Ljava/nio/Buffer;)V"), &[JValue::Object(&buffer)])?;
        env.call_method(&bitmap, jni_str!("recycle"), jni_sig!("()V"), &[])?;
        let array = env.call_method(&buffer, jni_str!("array"), jni_sig!("()[B"), &[])?.l()?;
        let array = env.cast_local::<jni::objects::JByteArray>(array)?;
        let bytes = env.convert_byte_array(&array)?;
        Ok(Some((bytes, size_px, size_px)))
    });
    match result {
        Ok(Some(icon)) => Some(icon),
        Ok(None) => {
            log::warn!("icon failed: {} (no such app or no icon)", entry.package);
            None
        }
        Err(e) => {
            log::warn!("icon failed: {} ({e})", entry.package);
            None
        }
    }
}

/// Open the app. `false` (and a log line) if Android refused, e.g. it was uninstalled.
pub fn launch(app: &AndroidApp, entry: &AppEntry) -> bool {
    let result = with_activity(app, |env, activity| {
        let la = launcher_apps(env, activity)?;
        let user = my_user(env)?;
        let comp = component(env, entry)?;
        env.call_method(
            &la,
            jni_str!("startMainActivity"),
            jni_sig!("(Landroid/content/ComponentName;Landroid/os/UserHandle;Landroid/graphics/Rect;Landroid/os/Bundle;)V"),
            &[JValue::Object(&comp), JValue::Object(&user), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())],
        )?;
        Ok(())
    });
    match result {
        Ok(()) => true,
        Err(e) => {
            log::warn!("launch failed: {} ({e})", entry.package);
            false
        }
    }
}
