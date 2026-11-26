//! Small pieces every app needs: quiet logging set from a system property,
//! clocks, and handing freed memory back to the system.

use std::ffi::CStr;

unsafe extern "C" {
    fn __system_property_get(name: *const libc::c_char, value: *mut libc::c_char) -> libc::c_int;
    fn mallopt(param: i32, value: i32) -> i32;
}

/// Drops one message: android-activity reports every refresh call as
/// "spurious", which would fill the log 60 times a second while moving.
struct QuietLogger(android_logger::AndroidLogger);

impl log::Log for QuietLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        self.0.enabled(m)
    }
    fn log(&self, r: &log::Record) {
        let spurious = r.target().starts_with("android_activity")
            && r.args().as_str().is_some_and(|s| s.starts_with("Spurious ALOOPER_POLL_CALLBACK"));
        if !spurious {
            self.0.log(r);
        }
    }
    fn flush(&self) {
        self.0.flush();
    }
}

/// Warnings only, unless the system property `prop` says `info` or `debug`
/// (`adb shell setprop debug.gallery.log info`). Read once, at start.
pub fn init_logging(tag: &str, prop: &CStr) {
    let level = match property(prop).as_deref() {
        Some("info") => log::LevelFilter::Info,
        Some("debug") => log::LevelFilter::Debug,
        _ => log::LevelFilter::Warn,
    };
    let logger = android_logger::AndroidLogger::new(
        android_logger::Config::default().with_max_level(level).with_tag(tag),
    );
    if log::set_logger(Box::leak(Box::new(QuietLogger(logger)))).is_ok() {
        log::set_max_level(level);
    }
}

/// A system property's value, or `None` when empty.
pub fn property(name: &CStr) -> Option<String> {
    let mut value = [0u8; 92];
    let n = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr().cast()) };
    let s = String::from_utf8_lossy(&value[..n.clamp(0, 91) as usize]).into_owned();
    (!s.is_empty()).then_some(s)
}

pub fn monotonic_ns(clock: libc::clockid_t) -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(clock, &mut ts) };
    ts.tv_sec * 1_000_000_000 + ts.tv_nsec
}

/// Time since this process started, from /proc (for start-up timing logs).
pub fn ms_since_process_start() -> f64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else { return -1.0 };
    // Field 22 (starttime, clock ticks since boot) comes after the ")" of the name.
    let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { return -1.0 };
    let Some(ticks) = rest.split_whitespace().nth(19).and_then(|v| v.parse::<f64>().ok()) else { return -1.0 };
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let now = monotonic_ns(libc::CLOCK_BOOTTIME) as f64 / 1e6;
    now - ticks / hz * 1e3
}

/// Ask the allocator to hand freed pages back about a second after they are
/// freed (bionic's M_DECAY_TIME, Android 9+), instead of keeping them.
pub fn return_freed_memory_quickly() {
    const M_DECAY_TIME: i32 = -100;
    unsafe { mallopt(M_DECAY_TIME, 1) };
}

/// Give freed memory back to the system now (bionic's M_PURGE, where supported).
pub fn purge_freed_memory() {
    const M_PURGE: i32 = -101;
    unsafe { mallopt(M_PURGE, 0) };
}
