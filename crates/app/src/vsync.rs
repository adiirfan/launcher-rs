//! Draws in step with the screen. Android calls back once per refresh, and
//! the main loop draws right then, so frames never drift behind the screen
//! the way a fixed sleep does.

use std::ffi::{c_long, c_void};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use android_activity::ndk_sys as sys;

/// A call has been asked for and has not come yet.
static POSTED: AtomicBool = AtomicBool::new(false);
/// When the latest refresh happened (monotonic clock, ns); 0 once taken.
static TICK_NS: AtomicI64 = AtomicI64::new(0);

pub struct Vsync {
    choreographer: *mut sys::AChoreographer,
}

impl Vsync {
    /// Must be made on the thread that polls for events: Android delivers the
    /// call while that thread waits.
    pub fn new() -> Option<Vsync> {
        // A loop from before an activity rebuild may have left a request
        // that will never be answered; start clean or none is ever sent.
        POSTED.store(false, Ordering::Release);
        TICK_NS.store(0, Ordering::Release);
        let choreographer = unsafe { sys::AChoreographer_getInstance() };
        (!choreographer.is_null()).then_some(Vsync { choreographer })
    }

    /// Ask for a call at the next refresh (once, however often this is called).
    pub fn request(&self) {
        if !POSTED.swap(true, Ordering::AcqRel) {
            unsafe { sys::AChoreographer_postFrameCallback(self.choreographer, Some(on_refresh), std::ptr::null_mut()) };
        }
    }

    /// The time of the refresh that arrived since the last call, if any.
    pub fn take_tick(&self) -> Option<i64> {
        Some(TICK_NS.swap(0, Ordering::AcqRel)).filter(|&t| t != 0)
    }
}

unsafe extern "C" fn on_refresh(frame_time_nanos: c_long, _data: *mut c_void) {
    POSTED.store(false, Ordering::Release);
    TICK_NS.store((frame_time_nanos as i64).max(1), Ordering::Release);
}
