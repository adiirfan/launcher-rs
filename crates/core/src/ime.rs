//! Keyboard text state as Rust sees it: positions in characters, not UTF-16.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImeState {
    pub text: String,
    /// Cursor or selection, start <= end, in characters.
    pub selection: (usize, usize),
    /// The word the keyboard is still composing, if any.
    pub compose: Option<(usize, usize)>,
}

impl ImeState {
    /// From Java's view: UTF-16 offsets, -1 meaning "none" for compose.
    pub fn from_java(text: &str, sel_start: i32, sel_end: i32, comp_start: i32, comp_end: i32) -> ImeState {
        let to_char = |utf16: i32| -> usize {
            let target = utf16.max(0) as usize;
            let mut units = 0;
            for (i, c) in text.chars().enumerate() {
                if units >= target {
                    return i;
                }
                units += c.len_utf16();
            }
            text.chars().count()
        };
        let ordered = |a: usize, b: usize| (a.min(b), a.max(b));
        let selection = ordered(to_char(sel_start), to_char(sel_end));
        let compose = if comp_start < 0 || comp_end < 0 {
            None
        } else {
            let c = ordered(to_char(comp_start), to_char(comp_end));
            (c.0 != c.1).then_some(c)
        };
        ImeState { text: text.to_string(), selection, compose }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_cursor_at_end() {
        let s = ImeState::from_java("hello", 5, 5, -1, -1);
        assert_eq!(s.text, "hello");
        assert_eq!(s.selection, (5, 5));
        assert_eq!(s.compose, None);
    }

    #[test]
    fn selection_and_compose() {
        let s = ImeState::from_java("hello", 2, 4, 0, 5);
        assert_eq!(s.selection, (2, 4));
        assert_eq!(s.compose, Some((0, 5)));
    }

    #[test]
    fn emoji_counts_as_one_character() {
        let s = ImeState::from_java("a😀b", 3, 3, -1, -1);
        assert_eq!(s.selection, (2, 2));
    }

    #[test]
    fn reversed_selection_is_ordered() {
        assert_eq!(ImeState::from_java("abc", 3, 1, -1, -1).selection, (1, 3));
    }

    #[test]
    fn out_of_range_is_clamped() {
        let s = ImeState::from_java("abc", 9, 9, 7, 2);
        assert_eq!(s.selection, (3, 3));
        assert_eq!(s.compose, Some((2, 3)));
    }

    #[test]
    fn empty_compose_is_none() {
        assert_eq!(ImeState::from_java("ab", 1, 1, 1, 1).compose, None);
    }
}
