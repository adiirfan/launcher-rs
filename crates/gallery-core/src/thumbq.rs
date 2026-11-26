//! Which thumbnails to load next. The grid says which are missing, most
//! wanted first; loaders take from the front. A newer list replaces the old
//! one, so after a fast fling nothing is loaded for rows long gone.

use std::collections::HashSet;

#[derive(Default)]
pub struct ThumbQueue {
    wanted: Vec<i64>,
    in_flight: HashSet<i64>,
    failed: HashSet<i64>,
}

impl ThumbQueue {
    /// The thumbnails missing now, most wanted first. Replaces the last list.
    pub fn want(&mut self, ids: &[i64]) {
        self.wanted.clear();
        self.wanted.extend_from_slice(ids);
    }

    /// The most wanted one nobody is loading and that hasn't failed.
    pub fn next(&mut self) -> Option<i64> {
        let pos = self.wanted.iter().position(|id| !self.in_flight.contains(id) && !self.failed.contains(id))?;
        let id = self.wanted.remove(pos);
        self.in_flight.insert(id);
        Some(id)
    }

    pub fn done(&mut self, id: i64, ok: bool) {
        self.in_flight.remove(&id);
        if !ok {
            self.failed.insert(id);
        }
    }

    /// Android had no thumbnail for it: draw a plain tile, don't ask again.
    pub fn failed(&self, id: i64) -> bool {
        self.failed.contains(&id)
    }

    /// The library changed (a file came back, an SD card returned): try again.
    pub fn clear_failures(&mut self) {
        self.failed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_want_wins() {
        let mut q = ThumbQueue::default();
        q.want(&[1, 2, 3]);
        assert_eq!(q.next(), Some(1));
        q.want(&[9, 8]);
        assert_eq!(q.next(), Some(9));
        assert_eq!(q.next(), Some(8));
        assert_eq!(q.next(), None);
    }

    #[test]
    fn failed_ids_not_requeued() {
        let mut q = ThumbQueue::default();
        q.want(&[9]);
        assert_eq!(q.next(), Some(9));
        q.done(9, false);
        q.want(&[9, 4]);
        assert_eq!(q.next(), Some(4));
        assert_eq!(q.next(), None);
        assert!(q.failed(9));
    }

    #[test]
    fn in_flight_not_twice() {
        let mut q = ThumbQueue::default();
        q.want(&[5, 6]);
        assert_eq!(q.next(), Some(5));
        q.want(&[5, 6]);
        assert_eq!(q.next(), Some(6));
        assert_eq!(q.next(), None);
        q.done(5, true);
        // Loaded and then pushed out of the atlas: wanted again, loads again.
        q.want(&[5]);
        assert_eq!(q.next(), Some(5));
    }

    #[test]
    fn forget_failures_after_a_library_change() {
        let mut q = ThumbQueue::default();
        q.want(&[3]);
        q.next();
        q.done(3, false);
        q.clear_failures();
        q.want(&[3]);
        assert_eq!(q.next(), Some(3));
    }
}
