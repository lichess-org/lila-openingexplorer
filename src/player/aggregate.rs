use std::collections::BTreeMap;

use nohash_hasher::IntMap;
use shakmaty::{
    ByColor, CastlingMode, Color, EnPassantMode, KnownOutcome, Position,
    packed::PackedUciMove,
    variant::{Variant, WithPositionMut},
    zobrist::Zobrist128,
};

use crate::{
    packed::{
        CellKey, ColdGroup, Month, PackedMovesDecoder, PlayerEntry, PlayerGroupHeader, PositionKey,
        Rating, color_bits,
    },
    player::{
        MAX_PLY,
        game::BorrowedPlayerGame,
        keys::{ColdBucket, MAX_COLD_BITS, PlayerGameIndex},
    },
};

/// The positions of the first `max_ply` plies of a game, each once, with
/// the move played from it the last time (removing loops). The final
/// position is not a visit.
pub fn replay(
    game: &BorrowedPlayerGame<'_>,
    max_ply: usize,
    scratch: &mut IntMap<Zobrist128, PackedUciMove>,
    out: &mut Vec<(PositionKey, PackedUciMove)>,
) {
    struct Replay<'a> {
        decoder: PackedMovesDecoder<'a>,
        max_ply: usize,
        visits: &'a mut IntMap<Zobrist128, PackedUciMove>,
    }

    impl WithPositionMut for Replay<'_> {
        type Output = ();

        fn call<P: Position>(mut self, pos: &mut P) {
            let mut hash: Zobrist128 = pos.zobrist_hash(EnPassantMode::Legal);
            let mut ply = 0;
            while ply < self.max_ply
                && let Some(m) = self.decoder.decode_next(pos).expect("valid moves")
            {
                self.visits
                    .insert(hash, PackedUciMove::pack(m.to_uci(CastlingMode::Chess960)));
                ply += 1;
                let updated = pos.update_zobrist_hash(hash, m, EnPassantMode::Legal);
                pos.play_unchecked(m);
                hash = updated.unwrap_or_else(|| pos.zobrist_hash(EnPassantMode::Legal));
            }
        }
    }

    let decoder = PackedMovesDecoder::new(game.moves).expect("valid moves");
    let mut pos = game.initial_position();
    let variant: Variant = pos.variant();
    scratch.clear();
    pos.with_position_mut(Replay {
        decoder,
        max_ply,
        visits: scratch,
    });
    out.extend(
        scratch
            .drain()
            .map(|(hash, m)| (PositionKey::from_zobrist(hash, variant), m)),
    );
}

/// The move played from the position in the game, if the game reached it
/// (the last time, like `replay`). Stops replaying at the first irreversible
/// move after a visit.
pub fn find_visit(
    game: &BorrowedPlayerGame<'_>,
    target: &PositionKey,
    max_ply: usize,
) -> Option<PackedUciMove> {
    struct Find<'a> {
        decoder: PackedMovesDecoder<'a>,
        max_ply: usize,
        variant: Variant,
        target: &'a PositionKey,
    }

    impl WithPositionMut for Find<'_> {
        type Output = Option<PackedUciMove>;

        fn call<P: Position>(mut self, pos: &mut P) -> Option<PackedUciMove> {
            let mut hash: Zobrist128 = pos.zobrist_hash(EnPassantMode::Legal);
            let mut found = None;
            let mut ply = 0;
            while ply < self.max_ply
                && let Some(m) = self.decoder.decode_next(pos).expect("valid moves")
            {
                if PositionKey::from_zobrist(hash, self.variant) == *self.target {
                    found = Some(PackedUciMove::pack(m.to_uci(CastlingMode::Chess960)));
                }
                if found.is_some() && pos.is_irreversible(m) {
                    // The position cannot occur again, so that was its
                    // last visit.
                    break;
                }
                ply += 1;
                let updated = pos.update_zobrist_hash(hash, m, EnPassantMode::Legal);
                pos.play_unchecked(m);
                hash = updated.unwrap_or_else(|| pos.zobrist_hash(EnPassantMode::Legal));
            }
            found
        }
    }

    let decoder = PackedMovesDecoder::new(game.moves).expect("valid moves");
    let mut pos = game.initial_position();
    let variant = pos.variant();
    pos.with_position_mut(Find {
        decoder,
        max_ply,
        variant,
        target,
    })
}

/// Positions with at least this many games of a player (and color) are
/// warm, the others cold. Any placement gives correct results (reads take
/// the union), so this only trades space against validation work: at 3
/// rather than 2, warm and cold together are 8% smaller, and 1.4% of reads
/// validate 2 more cold candidates.
const WARM_MIN_GAMES: usize = 3;

/// Cold items per group on average at most. Larger groups are smaller per
/// item (fewer keys, better compression) and cost little: one lookup reads
/// one group either way, and false candidates stay rare (about 0.3% of
/// lookups at 256).
const COLD_GROUP_ITEMS: u64 = 256;

/// Bucket bits for at most `COLD_GROUP_ITEMS` cold items per group on
/// average (the least bits with `items <= COLD_GROUP_ITEMS << bits`), up to
/// `MAX_COLD_BITS`.
pub fn cold_bits_for(items: u64) -> u8 {
    let groups = items.div_ceil(COLD_GROUP_ITEMS);
    let bits = u64::BITS - groups.saturating_sub(1).leading_zeros();
    bits.min(u32::from(MAX_COLD_BITS)) as u8
}

#[derive(Debug, Clone, Copy)]
struct Visit {
    color: Color,
    key: PositionKey,
    index: PlayerGameIndex,
    m: PackedUciMove,
    header: PlayerGroupHeader,
    month: Month,
    outcome: KnownOutcome,
    opponent_rating: Rating,
}

/// Visits of games, in memory.
#[derive(Default)]
pub struct Aggregation {
    visits: Vec<Visit>,
    scratch: IntMap<Zobrist128, PackedUciMove>,
    replayed: Vec<(PositionKey, PackedUciMove)>,
}

/// The cold groups of the touched buckets, by color.
pub type ColdGroups = Vec<(Color, ColdBucket, ColdGroup)>;

/// Warm entries and cold groups of an aggregation, in key order.
pub struct Placed {
    pub warm: Vec<(Color, PositionKey, PlayerEntry)>,
    /// The items of each touched bucket, with the bits after the bucket
    /// bits of the position key as suffixes (see `ColdBucket::of`). Two
    /// positions of a game with the same suffix have one item, and
    /// validating the game by replay finds either.
    pub cold: ColdGroups,
    pub cold_bits: ByColor<u8>,
    pub warm_visits: u64,
    pub cold_visits: u64,
}

impl Aggregation {
    pub fn new() -> Aggregation {
        Aggregation::default()
    }

    pub fn visits(&self) -> usize {
        self.visits.len()
    }

    pub fn add_game(&mut self, index: PlayerGameIndex, game: &BorrowedPlayerGame<'_>) {
        self.replayed.clear();
        replay(game, MAX_PLY, &mut self.scratch, &mut self.replayed);
        let header = PlayerGroupHeader {
            speed: game.speed,
            mode: game.mode,
        };
        self.visits
            .extend(self.replayed.iter().map(|&(key, m)| Visit {
                color: game.color,
                key,
                index,
                m,
                header,
                month: game.month,
                outcome: game.outcome,
                opponent_rating: game.opponent_rating,
            }));
    }

    /// Places the visits. `cold_bits` gives the bucket bits of a color from
    /// the number of its cold items.
    ///
    /// Positions are warm with at least `WARM_MIN_GAMES` games, with one
    /// exception that keeps warm and cold disjoint for every read: all
    /// visits of a game with the same cold item (color, bucket and suffix)
    /// are either cold or warm. Otherwise a read of a warm position would
    /// also find the game through the cold item of another of its
    /// positions with the same suffix, and validating the game by replay
    /// would count it a second time.
    pub fn place(mut self, cold_bits: impl Fn(Color, u64) -> u8) -> Placed {
        // In key order (color byte, then position key).
        self.visits
            .sort_unstable_by_key(|v| (color_bits(v.color), v.key, v.index));
        let mut is_warm = vec![false; self.visits.len()];
        let mut start = 0;
        for group in self
            .visits
            .chunk_by(|a, b| a.color == b.color && a.key == b.key)
        {
            if group.len() >= WARM_MIN_GAMES {
                is_warm[start..start + group.len()].fill(true);
            }
            start += group.len();
        }

        let mut counts = ByColor::new_with(|_| 0u64);
        for (v, &warm) in self.visits.iter().zip(&is_warm) {
            if !warm {
                *counts.get_mut(v.color) += 1;
            }
        }
        let bits =
            ByColor::new_with(|color| cold_bits(color, *counts.get(color)).min(MAX_COLD_BITS));

        // Visits of a game sharing a cold item go where any of them is warm.
        let mut items: Vec<(PlayerGameIndex, u8, ColdBucket, u16, usize)> = self
            .visits
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let (bucket, suffix) = ColdBucket::of(&v.key, *bits.get(v.color));
                (v.index, color_bits(v.color), bucket, suffix, i)
            })
            .collect();
        items.sort_unstable();
        for group in items.chunk_by(|a, b| a.0 == b.0 && a.1 == b.1 && a.2 == b.2 && a.3 == b.3) {
            if group.len() > 1 && group.iter().any(|f| is_warm[f.4]) {
                for f in group {
                    is_warm[f.4] = true;
                }
            }
        }

        let mut warm = Vec::new();
        let mut warm_visits = 0;
        let mut by_bucket: BTreeMap<(u8, ColdBucket), (Color, ColdGroup)> = BTreeMap::new();
        let mut cold_visits = 0;
        let mut start = 0;
        for group in self
            .visits
            .chunk_by(|a, b| a.color == b.color && a.key == b.key)
        {
            let first = group[0];
            let flags = &is_warm[start..start + group.len()];
            start += group.len();
            let mut entry = PlayerEntry::new();
            for (v, &warm) in group.iter().zip(flags) {
                if warm {
                    entry.add_game(
                        CellKey::new(v.m, v.header, v.month),
                        v.opponent_rating,
                        v.outcome,
                        v.index.into(),
                    );
                    warm_visits += 1;
                } else {
                    let (bucket, suffix) = ColdBucket::of(&v.key, *bits.get(v.color));
                    by_bucket
                        .entry((color_bits(v.color), bucket))
                        .or_insert_with(|| (v.color, ColdGroup::new()))
                        .1
                        .add(suffix, u64::from(v.index.0));
                    cold_visits += 1;
                }
            }
            if !entry.is_empty() {
                warm.push((first.color, first.key, entry));
            }
        }
        // In key order (color byte, then bucket).
        let cold: ColdGroups = by_bucket
            .into_iter()
            .map(|((_, bucket), (color, group))| (color, bucket, group))
            .collect();

        Placed {
            warm,
            cold,
            cold_bits: bits,
            warm_visits,
            cold_visits,
        }
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;
    use crate::player::game::PlayerGame;

    /// `find_visit` finds the visits of `replay`, and nothing else.
    #[test]
    fn test_find_visit() {
        arbtest(|u| {
            let game: PlayerGame = u.arbitrary()?;
            let game = game.as_borrowed();
            let max_ply = u.int_in_range(0..=100)?;
            let mut visits = Vec::new();
            replay(&game, max_ply, &mut IntMap::default(), &mut visits);
            for &(key, m) in &visits {
                assert_eq!(find_visit(&game, &key, max_ply), Some(m));
            }
            let other: PositionKey = u.arbitrary()?;
            if visits.iter().all(|&(key, _)| key != other) {
                assert_eq!(find_visit(&game, &other, max_ply), None);
            }
            Ok(())
        });
    }

    #[test]
    fn test_cold_bits_for() {
        assert_eq!(cold_bits_for(0), 0);
        assert_eq!(cold_bits_for(256), 0);
        assert_eq!(cold_bits_for(257), 1);
        assert_eq!(cold_bits_for(256 * 1024), 10);
        assert_eq!(cold_bits_for(20_000_000), 17);
        assert_eq!(cold_bits_for(u64::MAX), MAX_COLD_BITS);
    }
}
