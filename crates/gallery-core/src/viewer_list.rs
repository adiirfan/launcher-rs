//! Which picture the viewer shows after the list changes under it.

/// The same id if it's still there; if not, the one that came after it,
/// else the one before it; `None` when nothing is left.
pub fn index_after_removal(old: &[i64], current: i64, new: &[i64]) -> Option<usize> {
    if new.is_empty() {
        return None;
    }
    if let Some(i) = new.iter().position(|&id| id == current) {
        return Some(i);
    }
    let Some(at) = old.iter().position(|&id| id == current) else { return Some(0) };
    let find = |id: &i64| new.iter().position(|n| n == id);
    old[at + 1..].iter().find_map(find).or_else(|| old[..at].iter().rev().find_map(find)).or(Some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_index_after_removal() {
        let old = [1, 2, 3, 4];
        // Still there, even if it moved.
        assert_eq!(index_after_removal(&old, 3, &[3, 1, 2, 4]), Some(0));
        // Removed: the next one.
        assert_eq!(index_after_removal(&old, 2, &[1, 3, 4]), Some(1));
        // Removed with its neighbour: the first later one still there.
        assert_eq!(index_after_removal(&old, 2, &[1, 4]), Some(1));
        // The last removed: the one before.
        assert_eq!(index_after_removal(&old, 4, &[1, 2, 3]), Some(2));
        assert_eq!(index_after_removal(&old, 4, &[1, 3]), Some(1));
        // Everything gone.
        assert_eq!(index_after_removal(&old, 2, &[]), None);
        // Unknown current: start of the new list.
        assert_eq!(index_after_removal(&old, 9, &[7]), Some(0));
    }
}
