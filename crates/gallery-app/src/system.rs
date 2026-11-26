//! Asking Android to do things for us through the activity.

/// Seconds the phone's clock is ahead of UTC right now (daylight saving
/// included), and today's date there.
pub fn local_today() -> (gallery_core::dates::CivilDate, i32) {
    let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let t: libc::time_t = now_s as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let offset = if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() { 0 } else { tm.tm_gmtoff as i32 };
    (gallery_core::dates::civil_from_ms(now_s * 1000, offset), offset)
}
