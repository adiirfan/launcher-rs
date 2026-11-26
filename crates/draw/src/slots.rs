//! Equal-sized icon slots in one atlas. When every slot is taken, the slot
//! drawn longest ago (not in the last two frames) is reused, so a list can scroll past any number of
//! apps with a fixed amount of graphics memory.

use std::collections::HashMap;

const GAP: u32 = 1;

pub struct SlotPool {
    slot: u32,
    cols: u32,
    rows: u32,
    frame: u64,
    keys: HashMap<String, u32>,
    /// Per slot: who's in it, and the frame it was last drawn in.
    owners: Vec<Option<String>>,
    used: Vec<u64>,
}

impl SlotPool {
    pub fn new(atlas_w: u32, atlas_h: u32, slot: u32) -> SlotPool {
        let step = slot + GAP;
        let (cols, rows) = (atlas_w / step, atlas_h / step);
        let n = (cols * rows) as usize;
        SlotPool { slot, cols, rows, frame: 1, keys: HashMap::new(), owners: vec![None; n], used: vec![0; n] }
    }

    /// Atlas height (whole rows of slots) that holds `count` icons of
    /// `slot` px in an atlas `atlas_w` wide; at least one row.
    pub fn height_for(atlas_w: u32, count: usize, slot: u32) -> u32 {
        let step = slot + GAP;
        let cols = (atlas_w / step).max(1) as usize;
        (count.div_ceil(cols).max(1) as u32) * step
    }

    /// The atlas got taller (same width): add the new rows as empty slots.
    /// Slots are numbered row by row, so every icon keeps its place.
    pub fn grow(&mut self, atlas_h: u32) {
        let rows = atlas_h / (self.slot + GAP);
        if rows > self.rows {
            self.rows = rows;
            let n = (self.cols * self.rows) as usize;
            self.owners.resize(n, None);
            self.used.resize(n, 0);
        }
    }

    /// Some slot is empty, so inserting won't push another icon out.
    pub fn has_free(&self) -> bool {
        self.owners.iter().any(Option::is_none)
    }

    pub fn capacity(&self) -> usize {
        (self.cols * self.rows) as usize
    }

    pub fn slot_size(&self) -> u32 {
        self.slot
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    fn origin(&self, i: u32) -> (u32, u32) {
        let step = self.slot + GAP;
        ((i % self.cols) * step, (i / self.cols) * step)
    }

    pub fn get(&mut self, key: &str) -> Option<(u32, u32)> {
        let i = *self.keys.get(key)?;
        self.used[i as usize] = self.frame;
        Some(self.origin(i))
    }

    pub fn insert(&mut self, key: &str) -> Option<(u32, u32)> {
        if let Some(at) = self.get(key) {
            return Some(at);
        }
        let free = self.owners.iter().position(|o| o.is_none());
        let i = match free {
            Some(i) => i,
            None => {
                // Not one drawn this frame or the one before: new pictures
                // usually arrive before the frame is drawn, and pushing out
                // something just on screen makes it blink as it reloads.
                let (i, &used) = self.used.iter().enumerate().filter(|&(_, &u)| u + 1 < self.frame).min_by_key(|&(_, &u)| u)?;
                debug_assert!(used + 1 < self.frame);
                if let Some(old) = self.owners[i].take() {
                    self.keys.remove(&old);
                }
                i
            }
        };
        self.owners[i] = Some(key.to_string());
        self.used[i] = self.frame;
        self.keys.insert(key.to_string(), i as u32);
        Some(self.origin(i as u32))
    }

    pub fn forget(&mut self, key: &str) {
        if let Some(i) = self.keys.remove(key) {
            self.owners[i as usize] = None;
            self.used[i as usize] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_for_fits_every_icon_in_whole_rows() {
        // 168 px icons plus the 1 px gap: 12 a row in 2048 px.
        assert_eq!(SlotPool::height_for(2048, 121, 168), 11 * 169);
        assert_eq!(SlotPool::height_for(2048, 12, 168), 169);
        assert_eq!(SlotPool::height_for(2048, 13, 168), 2 * 169);
        assert_eq!(SlotPool::height_for(2048, 0, 168), 169);
    }

    #[test]
    fn a_pool_sized_by_height_for_holds_that_many() {
        let h = SlotPool::height_for(2048, 121, 168);
        assert!(SlotPool::new(2048, h, 168).capacity() >= 121);
    }

    #[test]
    fn grow_adds_rows_and_keeps_every_icon_in_place() {
        let mut p = SlotPool::new(2 * 11, 11, 10); // one row of two
        let a = p.insert("a").unwrap();
        let b = p.insert("b").unwrap();
        p.grow(3 * 11);
        assert_eq!(p.capacity(), 6);
        assert_eq!(p.get("a"), Some(a));
        assert_eq!(p.get("b"), Some(b));
        assert!(p.has_free());
        let c = p.insert("c").unwrap();
        assert!(c != a && c != b);
    }

    #[test]
    fn has_free_until_every_slot_is_taken() {
        let mut p = SlotPool::new(2 * 11, 11, 10); // two slots
        assert!(p.has_free());
        p.insert("a");
        assert!(p.has_free());
        p.insert("b");
        assert!(!p.has_free());
        p.forget("a");
        assert!(p.has_free());
    }

    #[test]
    fn insert_keeps_what_was_drawn_last_frame() {
        let mut p = SlotPool::new(2 * 11, 11, 10); // two slots
        p.insert("a").unwrap();
        p.insert("b").unwrap();
        p.begin_frame();
        p.get("a");
        p.get("b");
        // Next frame, before drawing: both were on screen a moment ago.
        p.begin_frame();
        assert!(p.insert("c").is_none());
        assert!(p.get("a").is_some() && p.get("b").is_some());
        // Only a is drawn; a frame later b may make way.
        p.begin_frame();
        p.begin_frame();
        p.get("a");
        p.begin_frame();
        assert!(p.insert("c").is_some());
        assert!(p.get("b").is_none() && p.get("a").is_some());
    }

    #[test]
    fn capacity_counts_whole_slots() {
        assert_eq!(SlotPool::new(2048, 1024, 150).capacity(), 13 * 6);
        assert_eq!(SlotPool::new(100, 100, 150).capacity(), 0);
    }

    #[test]
    fn slots_do_not_overlap() {
        let mut p = SlotPool::new(64, 32, 15);
        let mut seen = Vec::new();
        for i in 0..p.capacity() {
            let (x, y) = p.insert(&format!("k{i}")).unwrap();
            assert!(x + 15 <= 64 && y + 15 <= 32);
            assert!(!seen.contains(&(x, y)));
            seen.push((x, y));
        }
    }

    #[test]
    fn full_pool_reuses_the_least_recently_drawn() {
        let mut p = SlotPool::new(64, 16, 15);
        assert_eq!(p.capacity(), 4);
        for k in ["a", "b", "c", "d"] {
            p.insert(k).unwrap();
        }
        p.begin_frame();
        for k in ["a", "c", "d"] {
            p.get(k).unwrap();
        }
        p.begin_frame();
        let b_slot = {
            // "b" was drawn longest ago, so its slot goes to "e".
            let e = p.insert("e").unwrap();
            assert_eq!(p.get("b"), None);
            e
        };
        assert_eq!(p.get("e"), Some(b_slot));
        assert!(p.get("a").is_some());
    }

    #[test]
    fn a_slot_drawn_this_frame_is_never_taken() {
        let mut p = SlotPool::new(64, 16, 15);
        for k in ["a", "b", "c", "d"] {
            p.insert(k).unwrap();
        }
        p.begin_frame();
        for k in ["a", "b", "c", "d"] {
            p.get(k).unwrap();
        }
        assert_eq!(p.insert("e"), None);
        for k in ["a", "b", "c", "d"] {
            assert!(p.get(k).is_some());
        }
    }

    #[test]
    fn forget_frees_a_slot() {
        let mut p = SlotPool::new(64, 16, 15);
        for k in ["a", "b", "c", "d"] {
            p.insert(k).unwrap();
        }
        let b = p.get("b").unwrap();
        p.forget("b");
        assert_eq!(p.get("b"), None);
        assert_eq!(p.insert("e"), Some(b));
    }

    #[test]
    fn inserting_an_existing_key_returns_its_slot() {
        let mut p = SlotPool::new(64, 16, 15);
        let a = p.insert("a").unwrap();
        assert_eq!(p.insert("a"), Some(a));
    }
}
