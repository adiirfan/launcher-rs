//! Some phones deliver one Back press twice: as a key press and through
//! Android's newer back route. The second copy, arriving within moments,
//! would close one thing too many (often the whole app).

/// Presses closer together than this are one press.
pub const SAME_PRESS_MS: u64 = 300;

#[derive(Default)]
pub struct BackGate {
    last_ms: Option<u64>,
}

impl BackGate {
    pub const fn new() -> BackGate {
        BackGate { last_ms: None }
    }

    /// Whether a Back arriving at `now_ms` is a new press.
    pub fn pass(&mut self, now_ms: u64) -> bool {
        let fresh = self.last_ms.is_none_or(|t| now_ms.saturating_sub(t) >= SAME_PRESS_MS);
        if fresh {
            self.last_ms = Some(now_ms);
        }
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_doubled_press_counts_once() {
        let mut g = BackGate::default();
        assert!(g.pass(1000));
        assert!(!g.pass(1022));
    }

    #[test]
    fn separate_presses_all_count() {
        let mut g = BackGate::default();
        assert!(g.pass(1000));
        assert!(g.pass(1400));
        assert!(g.pass(1800));
    }
}
