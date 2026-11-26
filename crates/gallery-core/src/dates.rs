//! Calendar dates without a date library: days since 1970 and back, the
//! weekday, and the day headings the grid shows.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDate {
    pub y: i32,
    pub m: u8,
    pub d: u8,
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn days_from_civil(c: CivilDate) -> i64 {
    let y = c.y as i64 - if c.m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = c.m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + c.d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> CivilDate {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
    CivilDate { y, m, d }
}

/// The local date of a moment, given the local offset from UTC.
pub fn civil_from_ms(ms: i64, utc_offset_s: i32) -> CivilDate {
    civil_from_days(local_day(ms, utc_offset_s))
}

/// Local days since 1970 of a moment.
pub fn local_day(ms: i64, utc_offset_s: i32) -> i64 {
    (ms + utc_offset_s as i64 * 1000).div_euclid(86_400_000)
}

/// 0 = Monday … 6 = Sunday.
pub fn weekday(d: CivilDate) -> u8 {
    // 1970-01-01 was a Thursday.
    ((days_from_civil(d) + 3).rem_euclid(7)) as u8
}

const DAYS: [&str; 7] = ["MONDAY", "TUESDAY", "WEDNESDAY", "THURSDAY", "FRIDAY", "SATURDAY", "SUNDAY"];
const MONTHS: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

/// TODAY, YESTERDAY, "MONDAY 23 NOV", or with the year when it isn't this
/// year's: "FRIDAY 21 NOV 2025".
pub fn day_label(d: CivilDate, today: CivilDate) -> String {
    let (n, t) = (days_from_civil(d), days_from_civil(today));
    if n == t {
        return "TODAY".into();
    }
    if n == t - 1 {
        return "YESTERDAY".into();
    }
    let mut s = format!("{} {} {}", DAYS[weekday(d) as usize], d.d, MONTHS[(d.m as usize).clamp(1, 12) - 1]);
    if d.y != today.y {
        s.push_str(&format!(" {}", d.y));
    }
    s
}

/// "09:30", local time of day.
fn clock(ms: i64, utc_offset_s: i32) -> String {
    let minutes = (ms + utc_offset_s as i64 * 1000).div_euclid(60_000).rem_euclid(24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// "Monday" from "MONDAY", "Nov" from "NOV".
fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_string() + &c.as_str().to_lowercase()).unwrap_or_default()
}

fn long_day(d: CivilDate) -> String {
    format!("{} {} {}", title_case(DAYS[weekday(d) as usize]), d.d, title_case(MONTHS[(d.m as usize).clamp(1, 12) - 1]))
}

/// The viewer's top bar: "Today, 09:30", "Yesterday, 18:02",
/// "Monday 23 Nov, 09:30", "Friday 21 Nov 2025, 09:30".
pub fn short_when(ms: i64, utc_offset_s: i32, today: CivilDate) -> String {
    let d = civil_from_ms(ms, utc_offset_s);
    let (n, t) = (days_from_civil(d), days_from_civil(today));
    let day = if n == t {
        "Today".to_string()
    } else if n == t - 1 {
        "Yesterday".to_string()
    } else if d.y == today.y {
        long_day(d)
    } else {
        format!("{} {}", long_day(d), d.y)
    };
    format!("{day}, {}", clock(ms, utc_offset_s))
}

/// The INFO panel: "Thursday 26 Nov 2026, 09:30".
pub fn full_when(ms: i64, utc_offset_s: i32) -> String {
    let d = civil_from_ms(ms, utc_offset_s);
    format!("{} {}, {}", long_day(d), d.y, clock(ms, utc_offset_s))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-26 09:30 UTC, in milliseconds.
    const NOV26_0930: i64 = 1_795_685_400_000;

    #[test]
    fn viewer_dates() {
        let today = date(2026, 11, 26);
        assert_eq!(short_when(NOV26_0930, 0, today), "Today, 09:30");
        // Jakarta is seven hours ahead.
        assert_eq!(short_when(NOV26_0930, 7 * 3600, today), "Today, 16:30");
        assert_eq!(short_when(NOV26_0930 - 86_400_000 + 3_600_000 * 8 + 60_000 * 32, 0, today), "Yesterday, 18:02");
        assert_eq!(short_when(NOV26_0930 - 3 * 86_400_000, 0, today), "Monday 23 Nov, 09:30");
        assert_eq!(short_when(NOV26_0930 - 370 * 86_400_000, 0, today), "Friday 21 Nov 2025, 09:30");
        assert_eq!(full_when(NOV26_0930, 0), "Thursday 26 Nov 2026, 09:30");
        assert_eq!(full_when(NOV26_0930 - 9 * 3_600_000 - 25 * 60_000, 0), "Thursday 26 Nov 2026, 00:05");
    }

    fn date(y: i32, m: u8, d: u8) -> CivilDate {
        CivilDate { y, m, d }
    }

    #[test]
    fn days_round_trip() {
        for d in [date(1970, 1, 1), date(2000, 2, 29), date(2026, 11, 26), date(1969, 12, 31)] {
            assert_eq!(civil_from_days(days_from_civil(d)), d);
        }
        assert_eq!(days_from_civil(date(1970, 1, 2)), 1);
    }

    #[test]
    fn civil_from_ms_uses_the_offset() {
        // 2026-11-26 23:30 UTC is already the 27th at UTC+8.
        let ms = days_from_civil(date(2026, 11, 26)) * 86_400_000 + 23 * 3_600_000 + 30 * 60_000;
        assert_eq!(civil_from_ms(ms, 0), date(2026, 11, 26));
        assert_eq!(civil_from_ms(ms, 8 * 3600), date(2026, 11, 27));
        assert_eq!(civil_from_ms(-1, 0), date(1969, 12, 31));
    }

    #[test]
    fn weekday_known_dates() {
        assert_eq!(weekday(date(2026, 11, 26)), 3);
        assert_eq!(weekday(date(2026, 1, 1)), 3);
        assert_eq!(weekday(date(2026, 11, 23)), 0);
    }

    #[test]
    fn day_labels() {
        let today = date(2026, 11, 26);
        assert_eq!(day_label(today, today), "TODAY");
        assert_eq!(day_label(date(2026, 11, 25), today), "YESTERDAY");
        assert_eq!(day_label(date(2026, 11, 23), today), "MONDAY 23 NOV");
        assert_eq!(day_label(date(2026, 11, 9), today), "MONDAY 9 NOV");
        assert_eq!(day_label(date(2025, 11, 21), today), "FRIDAY 21 NOV 2025");
        // Yesterday across a year end.
        assert_eq!(day_label(date(2025, 12, 31), date(2026, 1, 1)), "YESTERDAY");
    }
}
