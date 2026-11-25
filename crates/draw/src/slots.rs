//! Equal-sized icon slots in one atlas. When every slot is taken, the slot
//! drawn longest ago is reused, so a list can scroll past any number of
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
                let (i, &used) = self.used.iter().enumerate().filter(|&(_, &u)| u < self.frame).min_by_key(|&(_, &u)| u)?;
                debug_assert!(used < self.frame);
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
