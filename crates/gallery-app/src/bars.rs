//! The status and navigation bar sizes, as Java reports them.

use std::sync::Mutex;

use jni::objects::JClass;
use jni::sys::jint;
use jni::EnvUnowned;
use launcher_core::geometry::Insets;

static BARS: Mutex<Option<[i32; 4]>> = Mutex::new(None);

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onInsets<'l>(
    _env: EnvUnowned<'l>,
    _class: JClass<'l>,
    top: jint,
    bottom: jint,
    left: jint,
    right: jint,
) {
    *BARS.lock().unwrap() = Some([top, bottom, left, right]);
    holo_platform::waker::wake();
}

/// Space the bars take. Before Android reports it, assume 24 dp and 48 dp.
pub fn insets(density: f32) -> Insets {
    match *BARS.lock().unwrap() {
        Some([top, bottom, left, right]) => {
            Insets { top: top as f32, bottom: bottom as f32, left: left as f32, right: right as f32 }
        }
        None => Insets { top: 24.0 * density, bottom: 48.0 * density, left: 0.0, right: 0.0 },
    }
}
