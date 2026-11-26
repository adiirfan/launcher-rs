//! Wakes the app's main loop from other threads and from Java callbacks.
//!
//! Android can rebuild the activity inside the same process (a font or
//! language change, sometimes after a long sleep). The new main loop must
//! call `init` again, or wake-ups would knock on a loop that no longer exists.

use std::sync::Mutex;

use android_activity::{AndroidApp, AndroidAppWaker};

static WAKER: Mutex<Option<AndroidAppWaker>> = Mutex::new(None);

/// Call at the start of every main loop.
pub fn init(app: &AndroidApp) {
    *WAKER.lock().unwrap() = Some(app.create_waker());
}

/// Wake the current main loop, if there is one.
pub fn wake() {
    if let Some(w) = WAKER.lock().unwrap().as_ref() {
        w.wake();
    }
}
