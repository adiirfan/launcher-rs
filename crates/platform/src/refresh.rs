//! Tells Android how fast we want the screen to refresh (Android 11+).
//! On older phones, or screens with one fixed rate, it does nothing.

use std::ffi::c_void;

use android_activity::AndroidApp;
use jni::objects::JObjectArray;
use jni::{jni_sig, jni_str};
use launcher_core::refresh::RateVote;
use ndk::native_window::NativeWindow;

use crate::jni_util::with_activity;

type SetFrameRate = unsafe extern "C" fn(window: *mut c_void, rate: f32, compatibility: i8) -> i32;

/// ANATIVEWINDOW_FRAME_RATE_COMPATIBILITY_DEFAULT
const COMPAT_DEFAULT: i8 = 0;

pub struct FrameRate {
    set: Option<SetFrameRate>,
    max_hz: f32,
}

impl FrameRate {
    pub fn new(app: &AndroidApp) -> FrameRate {
        let set = unsafe {
            let lib = libc::dlopen(c"libandroid.so".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            let sym = if lib.is_null() {
                std::ptr::null_mut()
            } else {
                libc::dlsym(lib, c"ANativeWindow_setFrameRate".as_ptr())
            };
            (!sym.is_null()).then(|| std::mem::transmute::<*mut c_void, SetFrameRate>(sym))
        };
        let max_hz = max_refresh_rate(app).unwrap_or(60.0);
        match set {
            Some(_) => log::info!("frame rate voting available, screen max {max_hz} Hz"),
            None => log::info!(
                "frame rate voting unavailable (Android {}), screen max {max_hz} Hz",
                android_release()
            ),
        }
        FrameRate { set, max_hz }
    }

    /// One refresh at the screen's top rate, in nanoseconds: the time a
    /// frame has while something moves (the launcher asks for the top rate).
    pub fn period_ns(&self) -> i64 {
        (1e9 / self.max_hz.max(1.0)) as i64
    }

    pub fn apply(&self, window: &NativeWindow, vote: RateVote) {
        let Some(set) = self.set else { return };
        let hz = match vote {
            RateVote::Max => self.max_hz,
            RateVote::Idle => 0.0,
        };
        let rc = unsafe { set(window.ptr().as_ptr().cast(), hz, COMPAT_DEFAULT) };
        log::info!("frame rate vote {vote:?} ({hz} Hz) -> {rc}");
    }
}

/// Highest rate among the display's supported modes.
fn max_refresh_rate(app: &AndroidApp) -> Option<f32> {
    with_activity(app, |env, activity| {
        let wm = env
            .call_method(activity, jni_str!("getWindowManager"), jni_sig!("()Landroid/view/WindowManager;"), &[])?
            .l()?;
        let display = env
            .call_method(&wm, jni_str!("getDefaultDisplay"), jni_sig!("()Landroid/view/Display;"), &[])?
            .l()?;
        let modes = env
            .call_method(&display, jni_str!("getSupportedModes"), jni_sig!("()[Landroid/view/Display$Mode;"), &[])?
            .l()?;
        let modes = env.cast_local::<JObjectArray>(modes)?;
        let n = modes.len(env)?;
        let mut best: f32 = 0.0;
        for i in 0..n {
            let mode = modes.get_element(env, i)?;
            let hz = env.call_method(&mode, jni_str!("getRefreshRate"), jni_sig!("()F"), &[])?.f()?;
            best = best.max(hz);
        }
        Ok((best > 0.0).then_some(best))
    })
    .unwrap_or_else(|e| {
        log::warn!("refresh rate query failed: {e}");
        None
    })
}

fn android_release() -> String {
    let mut buf = [0u8; 92];
    let n = unsafe { libc::__system_property_get(c"ro.build.version.release".as_ptr(), buf.as_mut_ptr().cast()) };
    String::from_utf8_lossy(&buf[..n.max(0) as usize]).into_owned()
}
