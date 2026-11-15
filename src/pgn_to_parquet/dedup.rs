use std::mem;

use rustc_hash::{FxBuildHasher, FxHashSet};

use crate::packed::GameId;

pub struct RecentGameIds {
    previous: FxHashSet<GameId>,
    current: FxHashSet<GameId>,
    generation_len: usize,
    duplicates: u64,
}

impl RecentGameIds {
    pub fn new() -> Self {
        // Enough to deal with historical timezone inconsistency, causing
        // March/April 2018 games to overlap (assuming the games of each
        // variant are tracked separately).
        Self::with_generation_len(100_000)
    }

    pub fn with_generation_len(generation_len: usize) -> Self {
        RecentGameIds {
            previous: FxHashSet::with_capacity_and_hasher(generation_len, FxBuildHasher),
            current: FxHashSet::with_capacity_and_hasher(generation_len, FxBuildHasher),
            generation_len,
            duplicates: 0,
        }
    }

    pub fn insert(&mut self, game_id: GameId) -> bool {
        if self.previous.contains(&game_id) || !self.current.insert(game_id) {
            self.duplicates += 1;
            return false;
        }

        if self.current.len() >= self.generation_len {
            mem::swap(&mut self.previous, &mut self.current);
            self.current.clear();
        }

        true
    }

    pub fn duplicates(&self) -> u64 {
        self.duplicates
    }
}

impl Default for RecentGameIds {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_id(s: &str) -> GameId {
        s.parse().expect("valid game id")
    }

    #[test]
    fn test_rejects_repeated_game_id() {
        let mut recent = RecentGameIds::new();

        assert!(recent.insert(game_id("abcdefgh")));
        assert!(recent.insert(game_id("ijklmnop")));
        assert!(!recent.insert(game_id("abcdefgh")));
        assert_eq!(recent.duplicates(), 1);
    }

    #[test]
    fn test_remembers_across_generation_rotation() {
        let mut recent = RecentGameIds::with_generation_len(2);

        assert!(recent.insert(game_id("00000000")));

        // Fill the current generation, so that it is rotated into the previous
        // one.
        assert!(recent.insert(game_id("11111111")));
        assert!(recent.insert(game_id("22222222")));

        assert!(!recent.insert(game_id("00000000")));
    }
}
