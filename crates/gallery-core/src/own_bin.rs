//! The gallery's own Bin, for Android 8 to 10, which has none: a deleted
//! file moves into the app's folder and one line records where it came
//! from and when. After 30 days it goes for good.

pub const KEEP_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinEntry {
    /// Where the file sits now, in the app's bin folder.
    pub stored: String,
    /// Where it came from, to put it back.
    pub original_path: String,
    pub deleted_ms: i64,
}

impl BinEntry {
    /// One line: stored, original, deleted, tab-separated.
    pub fn to_text(&self) -> String {
        format!("{}\t{}\t{}", self.stored, self.original_path, self.deleted_ms)
    }
}

/// Every well-formed line; broken ones are skipped.
pub fn from_text(text: &str) -> Vec<BinEntry> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let (stored, original, when) = (parts.next()?, parts.next()?, parts.next()?);
            if parts.next().is_some() || stored.is_empty() || original.is_empty() {
                return None;
            }
            Some(BinEntry { stored: stored.into(), original_path: original.into(), deleted_ms: when.parse().ok()? })
        })
        .collect()
}

pub fn to_text(entries: &[BinEntry]) -> String {
    entries.iter().map(|e| e.to_text() + "\n").collect()
}

pub fn expired(e: &BinEntry, now_ms: i64) -> bool {
    now_ms - e.deleted_ms >= KEEP_MS
}

/// Whole days until it goes for good (0 on the last day).
pub fn days_left(e: &BinEntry, now_ms: i64) -> i64 {
    ((e.deleted_ms + KEEP_MS - now_ms) / (24 * 3600 * 1000)).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> BinEntry {
        BinEntry {
            stored: "/storage/emulated/0/Android/data/dev.launcherrs.gallery/files/bin/1795685400000_IMG 1.jpg".into(),
            original_path: "/storage/emulated/0/DCIM/Camera/IMG 1.jpg".into(),
            deleted_ms: 1_795_685_400_000,
        }
    }

    #[test]
    fn bin_entry_round_trip() {
        let e = entry();
        let line = e.to_text();
        assert!(!line.contains('\n'));
        assert_eq!(from_text(&line), [e.clone()]);
        let two = vec![e.clone(), BinEntry { deleted_ms: 5, ..e.clone() }];
        assert_eq!(from_text(&to_text(&two)), two);
        // Broken lines (a crash mid-write) are skipped, the rest kept.
        let text = format!("{}\nnot a line\n{}\tx\n\n", e.to_text(), "a\tb");
        assert_eq!(from_text(&text), [e]);
    }

    #[test]
    fn expired_after_30_days() {
        let e = entry();
        let day = 24 * 3600 * 1000;
        assert!(!expired(&e, e.deleted_ms + 29 * day + 23 * 3600 * 1000));
        assert!(expired(&e, e.deleted_ms + 30 * day + 60_000));
        assert_eq!(days_left(&e, e.deleted_ms), 30);
        assert_eq!(days_left(&e, e.deleted_ms + 29 * day + 1), 0);
        assert_eq!(days_left(&e, e.deleted_ms + 31 * day), 0);
    }
}
