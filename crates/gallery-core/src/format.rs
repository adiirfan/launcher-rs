//! Numbers as the screens show them.

/// A video's length: 0:31, 1:04, 1:02:05.
pub fn duration(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "No items", "1 item", "2,031 items".
pub fn count(n: usize) -> String {
    match n {
        0 => "No items".into(),
        1 => "1 item".into(),
        _ => format!("{} items", grouped(n)),
    }
}

/// A file size in the units people know (1 MB = 1,000,000 bytes, as phones show).
pub fn bytes(n: u64) -> String {
    let f = n as f64;
    if n < 1_000 {
        format!("{n} B")
    } else if n < 1_000_000 {
        format!("{:.0} KB", f / 1e3)
    } else if n < 1_000_000_000 {
        format!("{:.1} MB", f / 1e6)
    } else {
        format!("{:.1} GB", f / 1e9)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(31_400), "0:31");
        assert_eq!(duration(64_000), "1:04");
        assert_eq!(duration(3_725_000), "1:02:05");
    }

    #[test]
    fn counts() {
        assert_eq!(count(0), "No items");
        assert_eq!(count(1), "1 item");
        assert_eq!(count(5), "5 items");
        assert_eq!(count(2031), "2,031 items");
        assert_eq!(count(1_234_567), "1,234,567 items");
    }

    #[test]
    fn byte_sizes() {
        assert_eq!(bytes(900), "900 B");
        assert_eq!(bytes(900_000), "900 KB");
        assert_eq!(bytes(4_100_000), "4.1 MB");
        assert_eq!(bytes(48_200_000), "48.2 MB");
        assert_eq!(bytes(1_200_000_000), "1.2 GB");
    }
}
