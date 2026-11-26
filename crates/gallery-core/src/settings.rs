//! The gallery's own settings, kept as `set <name> <value>` lines like the
//! launcher's.

use launcher_core::settings::Accent;

use crate::arrange::{Order, SortKey};

#[derive(Clone, Debug, PartialEq)]
pub struct GallerySettings {
    pub accent: Accent,
    /// Grid columns; 0 picks from the screen width.
    pub cols: u32,
    pub sort: SortKey,
    pub order: Order,
    /// Folders (album ids) kept out of Photos and Albums.
    pub hidden: Vec<i64>,
}

impl Default for GallerySettings {
    fn default() -> GallerySettings {
        GallerySettings { accent: Accent::Blue, cols: 0, sort: SortKey::Taken, order: Order::Desc, hidden: Vec::new() }
    }
}

/// The grid sizes the settings offer (0 = automatic).
pub const GRID_SIZES: [u32; 5] = [0, 3, 4, 5, 6];

impl GallerySettings {
    pub fn to_text(&self) -> String {
        let hidden: Vec<String> = self.hidden.iter().map(i64::to_string).collect();
        let mut out = format!(
            "set accent {}\nset cols {}\nset sort {}\nset order {}\n",
            self.accent.word(),
            self.cols,
            sort_word(self.sort),
            if self.order == Order::Asc { "asc" } else { "desc" },
        );
        if !hidden.is_empty() {
            out.push_str(&format!("set hidden {}\n", hidden.join(",")));
        }
        out
    }

    /// Unknown lines are skipped; a value that can't be read keeps its default.
    pub fn from_text(s: &str) -> GallerySettings {
        let mut out = GallerySettings::default();
        for line in s.lines() {
            let mut parts = line.split_whitespace();
            let (Some("set"), Some(name), Some(value), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
            match name {
                "accent" => {
                    if let Some(a) = Accent::ALL.into_iter().find(|a| a.word() == value) {
                        out.accent = a;
                    }
                }
                "cols" => {
                    if let Some(c) = value.parse().ok().filter(|c| GRID_SIZES.contains(c)) {
                        out.cols = c;
                    }
                }
                "sort" => {
                    if let Some(k) = [SortKey::Taken, SortKey::Changed, SortKey::Name, SortKey::Size].into_iter().find(|k| sort_word(*k) == value) {
                        out.sort = k;
                    }
                }
                "order" => match value {
                    "asc" => out.order = Order::Asc,
                    "desc" => out.order = Order::Desc,
                    _ => {}
                },
                "hidden" => out.hidden = value.split(',').filter_map(|v| v.parse().ok()).collect(),
                _ => {}
            }
        }
        out
    }
}

fn sort_word(k: SortKey) -> &'static str {
    match k {
        SortKey::Taken => "taken",
        SortKey::Changed => "changed",
        SortKey::Name => "name",
        SortKey::Size => "size",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let s = GallerySettings { accent: Accent::Green, cols: 5, sort: SortKey::Size, order: Order::Asc, hidden: vec![123, -4, 456] };
        assert_eq!(GallerySettings::from_text(&s.to_text()), s);
        let empty = GallerySettings::default();
        assert_eq!(GallerySettings::from_text(&empty.to_text()), empty);
    }

    #[test]
    fn missing_and_bad_values_keep_defaults() {
        assert_eq!(GallerySettings::from_text(""), GallerySettings::default());
        let s = GallerySettings::from_text("set accent pink\nset cols 9\nset cols x\nset sort colour\nset order sideways\nset wobble 1\nnonsense\nset order asc\n");
        assert_eq!(s, GallerySettings { order: Order::Asc, ..GallerySettings::default() });
        assert_eq!(GallerySettings::from_text("set cols 3\n").cols, 3);
        assert_eq!(GallerySettings::from_text("set accent orange\n").accent, Accent::Orange);
        assert_eq!(GallerySettings::from_text("set sort name\n").sort, SortKey::Name);
    }

    #[test]
    fn hidden_list_parses() {
        assert_eq!(GallerySettings::from_text("set hidden 123,456\n").hidden, vec![123, 456]);
        assert_eq!(GallerySettings::from_text("set hidden 7\n").hidden, vec![7]);
        // A bad entry is skipped, the rest kept.
        assert_eq!(GallerySettings::from_text("set hidden 1,x,2\n").hidden, vec![1, 2]);
        assert!(GallerySettings::from_text("set hidden\n").hidden.is_empty());
    }
}
