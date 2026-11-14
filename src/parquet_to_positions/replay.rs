use nohash_hasher::IntMap;
use shakmaty::{
    ByColor, CastlingMode, Color, EnPassantMode, KnownOutcome, Position,
    packed::PackedUciMove,
    variant::{Variant, VariantPosition, WithPositionMut},
    zobrist::Zobrist128,
};

use crate::{
    intermediate::IntermediateEntry,
    packed::{
        BorrowedLichessGame, GameIndex, GroupHeader, Month, PackedMovesDecoder, PositionKey,
        Rating, RatingGroup, Speed,
    },
};

/// A position reached in a game, and the move that was eventually played
/// from it.
#[derive(Copy, Clone)]
struct Visit {
    /// Ply at which the position was first reached.
    ply: usize,
    m: PackedUciMove,
}

/// The metadata shared by all records of one game.
#[derive(Copy, Clone)]
pub struct GameInfo {
    pub game_index: GameIndex,
    pub variant: Variant,
    pub month: Month,
    pub speed: Option<Speed>,
    pub ratings: ByColor<Rating>,
    pub rating_group: RatingGroup,
    pub outcome: KnownOutcome,
}

/// Replays games, with buffers reused across games.
pub struct Replayer {
    visits: IntMap<Zobrist128, Visit>,
    /// Side to move at ply 0 of the last replayed game.
    initial_turn: Color,
}

impl Default for Replayer {
    fn default() -> Replayer {
        Replayer::new()
    }
}

impl Replayer {
    pub fn new() -> Replayer {
        Replayer {
            visits: IntMap::default(),
            initial_turn: Color::White,
        }
    }

    /// Replays a game, and returns its metadata.
    pub fn replay_game(
        &mut self,
        game: &BorrowedLichessGame<'_>,
        game_index: GameIndex,
    ) -> GameInfo {
        let pos = game.initial_position();
        let info = GameInfo {
            game_index,
            variant: pos.variant(),
            month: game.date.earliest_month(),
            speed: game.speed,
            ratings: game.ratings_or_default(),
            rating_group: game.rating_group(),
            outcome: game.outcome,
        };
        self.replay(pos, game.moves);
        info
    }

    /// Replays the moves of a game from its initial position. Each position
    /// that a move was played from is recorded once, with the move that was
    /// played from it last: loops within the game are not counted multiple
    /// times. The final position is not recorded.
    fn replay(&mut self, mut pos: VariantPosition, moves: &[u8]) {
        self.visits.clear();
        self.initial_turn = pos.turn();

        struct ReplayGame<'a, 'b> {
            visits: &'a mut IntMap<Zobrist128, Visit>,
            decoder: PackedMovesDecoder<'b>,
        }

        impl WithPositionMut for ReplayGame<'_, '_> {
            type Output = ();

            fn call<P: Position>(mut self, pos: &mut P) -> Self::Output {
                let mut hash: Zobrist128 = pos.zobrist_hash(EnPassantMode::Legal);
                let mut ply = 0;
                while let Some(m) = self.decoder.decode_next(pos).expect("valid moves") {
                    let packed = PackedUciMove::pack(m.to_uci(CastlingMode::Chess960));
                    self.visits
                        .entry(hash)
                        .and_modify(|visit| visit.m = packed)
                        .or_insert(Visit { ply, m: packed });
                    ply += 1;

                    let updated = pos.update_zobrist_hash(hash, m, EnPassantMode::Legal);
                    pos.play_unchecked(m);
                    hash = updated.unwrap_or_else(|| pos.zobrist_hash(EnPassantMode::Legal));
                }
            }
        }

        pos.with_position_mut(ReplayGame {
            visits: &mut self.visits,
            decoder: PackedMovesDecoder::new(moves).expect("valid moves"),
        });
    }

    /// Number of positions recorded by the last replay.
    pub fn len(&self) -> usize {
        self.visits.len()
    }

    /// Side to move at the given ply of the last replayed game (there are
    /// no null moves).
    fn turn_at(&self, ply: usize) -> Color {
        self.initial_turn ^ !ply.is_multiple_of(2)
    }

    /// The records of the last replayed game.
    pub fn entries(&self, info: &GameInfo) -> impl Iterator<Item = IntermediateEntry> + '_ {
        let info = *info;
        let header = GroupHeader {
            speed: info.speed,
            rating_group: info.rating_group,
        };
        self.visits.iter().map(move |(&hash, visit)| {
            IntermediateEntry::new(
                PositionKey::from_zobrist(hash, info.variant),
                info.month,
                visit.m,
                header,
                info.ratings[self.turn_at(visit.ply)],
                info.outcome,
                info.game_index,
            )
        })
    }

    /// Recorded hashes of the last replayed game, of the positions first
    /// reached before the given ply.
    pub fn hashes_before(&self, ply: usize) -> impl Iterator<Item = Zobrist128> + '_ {
        self.visits
            .iter()
            .filter(move |(_, visit)| visit.ply < ply)
            .map(|(&hash, _)| hash)
    }
}

#[cfg(test)]
mod tests {
    use shakmaty::{Chess, san::San, uci::UciMove};

    use super::*;
    use crate::packed::{GamePlayer, LaxDate, LichessGame, PackedMoves, RatingMode};

    fn replay_san(sans: &str) -> Vec<(Zobrist128, String)> {
        let mut pos = VariantPosition::Chess(Chess::default());
        let initial = pos.clone();
        let mut encoder = PackedMoves::encoder();
        for san in sans.split_whitespace() {
            let m = san.parse::<San>().unwrap().to_move(&pos).unwrap();
            encoder.encode_next(&pos, m);
            pos.play_unchecked(m);
        }
        let moves = encoder.finish();

        let mut replayer = Replayer::default();
        replayer.replay(initial, &moves.0);
        let mut visits: Vec<_> = replayer.visits.into_iter().collect();
        visits.sort_by_key(|(_, visit)| visit.ply);
        visits
            .into_iter()
            .map(|(hash, visit)| (hash, visit.m.unpack().to_string()))
            .collect()
    }

    fn hash_after(sans: &str) -> Zobrist128 {
        let mut pos = Chess::default();
        for san in sans.split_whitespace() {
            let m = san.parse::<San>().unwrap().to_move(&pos).unwrap();
            pos.play_unchecked(m);
        }
        pos.zobrist_hash(EnPassantMode::Legal)
    }

    #[test]
    fn test_no_loops() {
        let records = replay_san("e4 e5 Nf3 Nc6");
        let moves: Vec<&str> = records.iter().map(|(_, m)| m.as_str()).collect();
        assert_eq!(moves, ["e2e4", "e7e5", "g1f3", "b8c6"]);
        assert_eq!(records[0].0, hash_after(""));
        assert_eq!(records[3].0, hash_after("e4 e5 Nf3"));
    }

    #[test]
    fn test_loop_keeps_last_move() {
        // P -Nf3-> Q -Nf6-> . -Ng1-> . -Ng8-> P -e4-> ...
        let records = replay_san("Nf3 Nf6 Ng1 Ng8 e4 e5");
        let moves: Vec<&str> = records.iter().map(|(_, m)| m.as_str()).collect();
        // The starting position keeps the move eventually played (e4), the
        // positions inside the loop are still recorded.
        assert_eq!(moves, ["e2e4", "g8f6", "f3g1", "f6g8", "e7e5"]);
        assert_eq!(records[0].0, hash_after(""));
        assert_eq!(records[4].0, hash_after("e4"));
    }

    #[test]
    fn test_repetition_at_end() {
        // Threefold repetition: the final position is never recorded, but
        // the positions of the cycle keep their last moves.
        let records = replay_san("Nf3 Nf6 Ng1 Ng8 Nf3 Nf6 Ng1 Ng8");
        let moves: Vec<&str> = records.iter().map(|(_, m)| m.as_str()).collect();
        assert_eq!(moves, ["g1f3", "g8f6", "f3g1", "f6g8"]);
    }

    #[test]
    fn test_nested_loops() {
        // The inner loop returns to the position after Nf3 Nf6, the outer
        // loop to the starting position.
        let records = replay_san("Nf3 Nf6 Nc3 Nc6 Nb1 Nb8 Ng1 Ng8 d4");
        let moves: Vec<&str> = records.iter().map(|(_, m)| m.as_str()).collect();
        assert_eq!(
            moves,
            ["d2d4", "g8f6", "f3g1", "b8c6", "c3b1", "c6b8", "f6g8"]
        );
    }

    #[test]
    fn test_game_rating_group() {
        let mut pos = Chess::default();
        let mut encoder = PackedMoves::encoder();
        for san in "e4 e5 Nf3 Nc6 Bb5".split_whitespace() {
            let m = san.parse::<San>().unwrap().to_move(&pos).unwrap();
            encoder.encode_next(&VariantPosition::Chess(pos.clone()), m);
            pos.play_unchecked(m);
        }
        let player = |name: &str, rating: i32| GamePlayer {
            title: None,
            name: name.to_owned(),
            rating: Some(Rating::try_from(rating).unwrap()),
        };
        let game = LichessGame {
            players: ByColor {
                white: player("white", 2600),
                black: player("black", 2300),
            },
            date: LaxDate::from_ymd(2026, Some(11), Some(14)).unwrap(),
            outcome: KnownOutcome::Draw,
            speed: Some(Speed::Blitz),
            rating_mode: RatingMode::Rated,
            setup: None,
            event: None,
            moves: encoder.finish(),
        };

        let mut replayer = Replayer::default();
        let info = replayer.replay_game(&game.as_borrowed(), GameIndex::new(7).unwrap());
        let mut entries: Vec<_> = replayer
            .entries(&info)
            .map(|entry| {
                (
                    entry.uci_move().unpack().to_string(),
                    entry.group_header().rating_group,
                    i32::from(entry.rating()),
                )
            })
            .collect();
        entries.sort();

        assert_eq!(
            entries,
            [
                ("b8c6".to_owned(), RatingGroup::Group2200, 2300),
                ("e2e4".to_owned(), RatingGroup::Group2200, 2600),
                ("e7e5".to_owned(), RatingGroup::Group2200, 2300),
                ("f1b5".to_owned(), RatingGroup::Group2200, 2600),
                ("g1f3".to_owned(), RatingGroup::Group2200, 2600),
            ]
        );
    }

    #[test]
    fn test_castling_uci() {
        let records = replay_san("e4 e5 Nf3 Nc6 Bc4 Bc5 O-O");
        assert_eq!(
            records.last().unwrap().1.parse::<UciMove>().unwrap(),
            "e1h1".parse::<UciMove>().unwrap()
        );
    }
}
