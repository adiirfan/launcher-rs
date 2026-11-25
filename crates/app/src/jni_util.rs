//! Getting a JNI environment and the activity object from android-activity.

use android_activity::AndroidApp;
use jni::objects::JObject;
use jni::{Env, JavaVM};

/// Run `f` with a JNI env (current thread attached) and our activity.
/// A Java exception inside `f` comes back as an `Err` and is cleared.
pub fn with_activity<R>(
    app: &AndroidApp,
    f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<R>,
) -> jni::errors::Result<R> {
    // Safety: android-activity owns a valid JavaVM and a global ref to the
    // activity for the app's whole life.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    vm.attach_current_thread(|env| {
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        f(env, &activity)
    })
}
