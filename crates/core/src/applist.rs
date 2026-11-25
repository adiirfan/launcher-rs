//! The saved list of installed apps, so Home can draw before asking Android.

use crate::home::AppKey;

#[derive(Clone, Debug, PartialEq)]
pub struct AppRecord {
    pub key: AppKey,
    pub label: String,
}

/// Header `v1 <seq>`, one `package\tactivity\tlabel` line per app, then `end`.
pub fn to_text(records: &[AppRecord], seq: i32) -> String {
    let mut out = format!("v1 {seq}\n");
    for r in records {
        out.push_str(&escape(&r.key.package));
        out.push('\t');
        out.push_str(&escape(&r.key.activity));
        out.push('\t');
        out.push_str(&escape(&r.label));
        out.push('\n');
    }
    out.push_str("end\n");
    out
}

pub fn from_text(s: &str) -> Option<(Vec<AppRecord>, i32)> {
    let mut lines = s.lines();
    let seq = lines.next()?.strip_prefix("v1 ")?.trim().parse().ok()?;
    let mut records = Vec::new();
    for line in lines {
        if line == "end" {
            return Some((records, seq));
        }
        let mut parts = line.split('\t');
        let (p, a, l) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || p.is_empty() || a.is_empty() {
            return None;
        }
        records.push(AppRecord { key: AppKey::new(&unescape(p)?, &unescape(a)?), label: unescape(l)? });
    }
    None
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            '\\' => '\\',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(p: &str, label: &str) -> AppRecord {
        AppRecord { key: AppKey::new(p, &format!("{p}.Main")), label: label.to_string() }
    }

    #[test]
    fn round_trip_with_awkward_labels() {
        let list = vec![rec("com.a", "Camera"), rec("com.b", "Tab\there\nnew line \\ slash"), rec("com.c", "日本語"), rec("com.d", "")];
        let text = to_text(&list, 42);
        assert_eq!(text.lines().count(), 6, "header, four apps, end marker");
        assert_eq!(from_text(&text), Some((list, 42)));
    }

    #[test]
    fn empty_list_round_trips() {
        assert_eq!(from_text(&to_text(&[], -1)), Some((vec![], -1)));
    }

    #[test]
    fn broken_files_are_none() {
        assert_eq!(from_text(""), None);
        assert_eq!(from_text("garbage"), None);
        assert_eq!(from_text("v1 x\n"), None);
        assert_eq!(from_text("v1 3\ncom.a\tonly-two\n"), None);
        let mut text = to_text(&[rec("com.a", "A")], 3);
        text.truncate(text.len() - 4);
        assert_eq!(from_text(&text), None, "a cut-off file has no end marker");
    }
}
