//! Installed apps: list them, load an icon, launch one.

use android_activity::AndroidApp;
use jni::objects::{JObject, JString, JValue};
use jni::{Env, jni_sig, jni_str};

use launcher_core::applist::AppRecord;
use launcher_core::geometry::{HOLO_COLS, HOLO_ROWS};
use launcher_core::home::{AppKey, HomeLayout};

use launcher_draw::shape::round_square_mask;

use crate::jni_util::with_activity;

/// Corner radius of Holo's rounded-square icons, as a share of the icon size.
const HOLO_CORNER: f32 = 0.18;

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
pub fn list_apps(app: &AndroidApp) -> Vec<AppRecord> {
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
            let entry = env.with_local_frame(FRAME, |env| -> jni::errors::Result<AppRecord> {
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
                Ok(AppRecord {
                    label: java_string(env, &label)?,
                    key: AppKey { package: java_string(env, &pkg)?, activity: java_string(env, &cls)? },
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

fn component<'l>(env: &mut Env<'l>, entry: &AppKey) -> jni::errors::Result<JObject<'l>> {
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
pub fn load_icon_rgba(app: &AndroidApp, entry: &AppKey, size_px: i32) -> Option<(Vec<u8>, i32, i32)> {
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
        // Two-layer icons (Android 8+): draw both layers full-bleed and cut
        // them to Holo's rounded square ourselves, instead of the phone's
        // own mask. Each layer is 1.5x the visible part, so grow the bounds
        // by a quarter of the size on every side.
        let layered = env.is_instance_of(&drawable, jni_str!("android/graphics/drawable/AdaptiveIconDrawable"))?;
        if layered {
            let pad = size_px / 4;
            for getter in [jni_str!("getBackground"), jni_str!("getForeground")] {
                let layer = env.call_method(&drawable, getter, jni_sig!("()Landroid/graphics/drawable/Drawable;"), &[])?.l()?;
                if layer.is_null() {
                    continue;
                }
                env.call_method(
                    &layer,
                    jni_str!("setBounds"),
                    jni_sig!("(IIII)V"),
                    &[JValue::Int(-pad), JValue::Int(-pad), JValue::Int(size_px + pad), JValue::Int(size_px + pad)],
                )?;
                env.call_method(&layer, jni_str!("draw"), jni_sig!("(Landroid/graphics/Canvas;)V"), &[JValue::Object(&canvas)])?;
            }
        } else {
            env.call_method(
                &drawable,
                jni_str!("setBounds"),
                jni_sig!("(IIII)V"),
                &[JValue::Int(0), JValue::Int(0), JValue::Int(size_px), JValue::Int(size_px)],
            )?;
            env.call_method(&drawable, jni_str!("draw"), jni_sig!("(Landroid/graphics/Canvas;)V"), &[JValue::Object(&canvas)])?;
        }
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
        let mut bytes = env.convert_byte_array(&array)?;
        if layered {
            round_square_mask(&mut bytes, size_px as u32, size_px as f32 * HOLO_CORNER);
        }
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
pub fn launch(app: &AndroidApp, entry: &AppKey) -> bool {
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

/// One of an app's own shortcuts ("New tab", "Incognito tab" for Chrome).
#[derive(Clone, Debug, PartialEq)]
pub struct Shortcut {
    pub id: String,
    pub label: String,
    pub package: String,
}

/// Up to four shortcuts for the app. Empty when it has none, or when
/// Android only answers the default Home and we're not it.
pub fn shortcuts(app: &AndroidApp, entry: &AppKey) -> Vec<Shortcut> {
    let result = with_activity(app, |env, activity| {
        let pkg = env.new_string(&entry.package)?;
        let text = env
            .call_method(activity, jni_str!("shortcuts"), jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"), &[JValue::Object(&pkg)])?
            .l()?;
        let text = env.cast_local::<JString>(text)?;
        Ok(String::from(text.mutf8_chars(env)?))
    });
    match result {
        Ok(text) => text
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(id, label)| Shortcut { id: id.to_string(), label: label.to_string(), package: entry.package.clone() })
            .collect(),
        Err(e) => {
            log::warn!("shortcuts failed: {} ({e})", entry.package);
            Vec::new()
        }
    }
}

pub fn start_shortcut(app: &AndroidApp, s: &Shortcut) -> bool {
    let result = with_activity(app, |env, activity| {
        let pkg = env.new_string(&s.package)?;
        let id = env.new_string(&s.id)?;
        env.call_method(
            activity,
            jni_str!("startShortcut"),
            jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Z"),
            &[JValue::Object(&pkg), JValue::Object(&id)],
        )?
        .z()
    });
    match result {
        Ok(ok) => ok,
        Err(e) => {
            log::warn!("start shortcut failed: {} ({e})", s.package);
            false
        }
    }
}

/// The app's page in Settings.
pub fn app_info(app: &AndroidApp, entry: &AppKey) -> bool {
    let result = with_activity(app, |env, activity| {
        let la = launcher_apps(env, activity)?;
        let user = my_user(env)?;
        let comp = component(env, entry)?;
        env.call_method(
            &la,
            jni_str!("startAppDetailsActivity"),
            jni_sig!("(Landroid/content/ComponentName;Landroid/os/UserHandle;Landroid/graphics/Rect;Landroid/os/Bundle;)V"),
            &[JValue::Object(&comp), JValue::Object(&user), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())],
        )?;
        Ok(())
    });
    match result {
        Ok(()) => true,
        Err(e) => {
            log::warn!("app info failed: {} ({e})", entry.package);
            false
        }
    }
}

/// Android's "uninstall this app?" prompt.
pub fn uninstall(app: &AndroidApp, entry: &AppKey) -> bool {
    crate::system::start(app, "android.intent.action.DELETE", Some(&format!("package:{}", entry.package)))
}

fn package_manager<'l>(env: &mut Env<'l>, activity: &JObject) -> jni::errors::Result<JObject<'l>> {
    env.call_method(activity, jni_str!("getPackageManager"), jni_sig!("()Landroid/content/pm/PackageManager;"), &[])?
        .l()
}

/// How many times the phone has booted. Android's change counter restarts
/// at every boot, so a saved counter is only meaningful within one boot.
pub fn boot_count(app: &AndroidApp) -> i32 {
    with_activity(app, |env, activity| {
        let resolver = env
            .call_method(activity, jni_str!("getContentResolver"), jni_sig!("()Landroid/content/ContentResolver;"), &[])?
            .l()?;
        let name = env.new_string("boot_count")?;
        env.call_static_method(
            jni_str!("android/provider/Settings$Global"),
            jni_str!("getInt"),
            jni_sig!("(Landroid/content/ContentResolver;Ljava/lang/String;I)I"),
            &[JValue::Object(&resolver), JValue::Object(&name), JValue::Int(-1)],
        )?
        .i()
    })
    .unwrap_or(-1)
}

/// Android's change counter right now, with the packages installed, updated
/// or removed since `seq`. `None` when nothing changed.
pub fn changed_since(app: &AndroidApp, seq: i32) -> Option<(i32, Vec<String>)> {
    let result = with_activity(app, |env, activity| {
        let pm = package_manager(env, activity)?;
        let changed = env
            .call_method(&pm, jni_str!("getChangedPackages"), jni_sig!("(I)Landroid/content/pm/ChangedPackages;"), &[JValue::Int(seq)])?
            .l()?;
        if changed.is_null() {
            return Ok(None);
        }
        let new_seq = env.call_method(&changed, jni_str!("getSequenceNumber"), jni_sig!("()I"), &[])?.i()?;
        let names = env.call_method(&changed, jni_str!("getPackageNames"), jni_sig!("()Ljava/util/List;"), &[])?.l()?;
        let n = env.call_method(&names, jni_str!("size"), jni_sig!("()I"), &[])?.i()?;
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let name = env.with_local_frame(4, |env| -> jni::errors::Result<String> {
                let s = env.call_method(&names, jni_str!("get"), jni_sig!("(I)Ljava/lang/Object;"), &[JValue::Int(i)])?.l()?;
                java_string(env, &s)
            })?;
            out.push(name);
        }
        Ok(Some((new_seq, out)))
    });
    result.unwrap_or_else(|e| {
        log::warn!("changed packages: {e}");
        None
    })
}

/// Android's current change counter (what to save after a full listing).
pub fn current_seq(app: &AndroidApp) -> i32 {
    changed_since(app, 0).map(|(s, _)| s).unwrap_or(0)
}

/// The package Android would open for `intent` by default; when there's no
/// default yet (the "open with" chooser), the first app that can handle it.
fn default_package(env: &mut Env, pm: &JObject, intent: &JObject) -> jni::errors::Result<Option<String>> {
    const MATCH_DEFAULT_ONLY: i32 = 0x0001_0000;
    let info = env
        .call_method(
            pm,
            jni_str!("resolveActivity"),
            jni_sig!("(Landroid/content/Intent;I)Landroid/content/pm/ResolveInfo;"),
            &[JValue::Object(intent), JValue::Int(MATCH_DEFAULT_ONLY)],
        )?
        .l()?;
    let mut pkg = package_of(env, &info)?;
    if pkg.as_deref().is_none_or(|p| p == "android") {
        let list = env
            .call_method(
                pm,
                jni_str!("queryIntentActivities"),
                jni_sig!("(Landroid/content/Intent;I)Ljava/util/List;"),
                &[JValue::Object(intent), JValue::Int(MATCH_DEFAULT_ONLY)],
            )?
            .l()?;
        let n = env.call_method(&list, jni_str!("size"), jni_sig!("()I"), &[])?.i()?;
        pkg = None;
        if n > 0 {
            let first = env.call_method(&list, jni_str!("get"), jni_sig!("(I)Ljava/lang/Object;"), &[JValue::Int(0)])?.l()?;
            pkg = package_of(env, &first)?;
        }
    }
    Ok(pkg)
}

fn package_of(env: &mut Env, info: &JObject) -> jni::errors::Result<Option<String>> {
    if info.is_null() {
        return Ok(None);
    }
    let ai = env
        .get_field(info, jni_str!("activityInfo"), jni_sig!("Landroid/content/pm/ActivityInfo;"))?
        .l()?;
    if ai.is_null() {
        return Ok(None);
    }
    let name = env.get_field(&ai, jni_str!("packageName"), jni_sig!("Ljava/lang/String;"))?.l()?;
    Ok(Some(java_string(env, &name)?))
}

/// Phone and Messages left of the all-apps button, Browser and Camera right
/// of it: whatever apps the phone uses for those by default, matched to
/// their launcher entry in `installed`.
pub fn default_dock(app: &AndroidApp, installed: &[AppRecord]) -> [Option<AppKey>; 5] {
    let wanted: [(usize, &str, Option<&str>); 4] = [
        (0, "android.intent.action.DIAL", None),
        (1, "android.intent.action.SENDTO", Some("smsto:")),
        (3, "android.intent.action.VIEW", Some("https://example.com")),
        (4, "android.media.action.STILL_IMAGE_CAMERA", None),
    ];
    let packages = with_activity(app, |env, activity| {
        let pm = package_manager(env, activity)?;
        let mut out: Vec<(usize, Option<String>)> = Vec::new();
        for (slot, action, data) in wanted {
            let pkg = env.with_local_frame(16, |env| -> jni::errors::Result<Option<String>> {
                let action = env.new_string(action)?;
                let intent = env.new_object(
                    jni_str!("android/content/Intent"),
                    jni_sig!("(Ljava/lang/String;)V"),
                    &[JValue::Object(&action)],
                )?;
                if let Some(data) = data {
                    let s = env.new_string(data)?;
                    let uri = env
                        .call_static_method(jni_str!("android/net/Uri"), jni_str!("parse"), jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"), &[JValue::Object(&s)])?
                        .l()?;
                    env.call_method(&intent, jni_str!("setData"), jni_sig!("(Landroid/net/Uri;)Landroid/content/Intent;"), &[JValue::Object(&uri)])?;
                }
                default_package(env, &pm, &intent)
            });
            // One intent failing (no camera, say) shouldn't lose the others.
            out.push((slot, pkg.unwrap_or(None)));
        }
        Ok(out)
    })
    .unwrap_or_default();
    let mut dock: [Option<AppKey>; 5] = Default::default();
    for (slot, pkg) in packages {
        dock[slot] = pkg.and_then(|p| installed.iter().find(|r| r.key.package == p).map(|r| r.key.clone()));
    }
    dock
}

/// Starts a layout for a first run: an empty first page (the clock carries
/// it) and the phone's default apps in the dock.
pub fn first_run_layout(app: &AndroidApp, installed: &[AppRecord]) -> HomeLayout {
    let mut layout = HomeLayout::new(HOLO_COLS, HOLO_ROWS);
    layout.dock = default_dock(app, installed);
    layout
}
