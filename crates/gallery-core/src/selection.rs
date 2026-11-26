//! Which items are chosen in selection mode. A long press then a drag
//! chooses the whole run between, as in Google Photos; dragging back
//! un-chooses what the drag added but keeps what was chosen before it.

use std::collections::HashSet;

#[derive(Debug, Default, Clone)]
pub struct Selection {
    chosen: HashSet<i64>,
    /// What was chosen when the current drag began.
    before_drag: Option<HashSet<i64>>,
}

impl Selection {
    pub fn toggle(&mut self, id: i64) {
        if !self.chosen.remove(&id) {
            self.chosen.insert(id);
        }
    }

    pub fn contains(&self, id: i64) -> bool {
        self.chosen.contains(&id)
    }

    /// Choose the run from `from` to `to` (positions in `ids`, either way
    /// round) on top of what was chosen when the drag began.
    pub fn drag_range(&mut self, ids: &[i64], from: usize, to: usize) {
        if from >= ids.len() {
            return;
        }
        let to = to.min(ids.len() - 1);
        let base = self.before_drag.get_or_insert_with(|| self.chosen.clone()).clone();
        let (a, b) = (from.min(to), from.max(to));
        self.chosen = base;
        self.chosen.extend(&ids[a..=b]);
    }

    /// The finger lifted: the next drag starts from what's chosen now.
    pub fn end_drag(&mut self) {
        self.before_drag = None;
    }

    pub fn select_all(&mut self, ids: &[i64]) {
        self.chosen.extend(ids);
    }

    /// Chosen ids, smallest first.
    pub fn ids(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self.chosen.iter().copied().collect();
        v.sort_unstable();
        v
    }

    pub fn count(&self) -> usize {
        self.chosen.len()
    }

    pub fn clear(&mut self) {
        self.chosen.clear();
        self.before_drag = None;
    }

    /// Forget ids that are no longer in the library.
    pub fn retain(&mut self, alive: impl Fn(i64) -> bool) {
        self.chosen.retain(|&id| alive(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_selects_run_and_shrinks_back() {
        let ids: Vec<i64> = (10..20).collect();
        let mut s = Selection::default();
        s.toggle(11);
        s.drag_range(&ids, 3, 6);
        assert_eq!(s.ids(), [11, 13, 14, 15, 16]);
        // Back towards the start: what the drag added shrinks, 11 stays.
        s.drag_range(&ids, 3, 4);
        assert_eq!(s.ids(), [11, 13, 14]);
        // Past the start the other way.
        s.drag_range(&ids, 3, 0);
        assert_eq!(s.ids(), [10, 11, 12, 13]);
        s.end_drag();
        s.drag_range(&ids, 8, 9);
        assert_eq!(s.ids(), [10, 11, 12, 13, 18, 19]);
        assert_eq!(s.count(), 6);
        // Positions past the end are ignored.
        s.end_drag();
        s.drag_range(&ids, 9, 40);
        assert_eq!(s.count(), 6);
    }

    #[test]
    fn toggle_twice_clears() {
        let mut s = Selection::default();
        s.toggle(5);
        assert!(s.contains(5));
        s.toggle(5);
        assert!(!s.contains(5));
        assert_eq!(s.count(), 0);
        s.select_all(&[1, 2, 3]);
        assert_eq!(s.count(), 3);
        s.retain(|id| id != 2);
        assert_eq!(s.ids(), [1, 3]);
        s.clear();
        assert!(s.ids().is_empty());
    }
}
