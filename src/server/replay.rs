use shakmaty::{
    CastlingMode, Color, EnPassantMode, Position, packed::PackedUciMove, variant::WithPositionMut,
    zobrist::Zobrist128,
};

use crate::packed::{BorrowedLichessGame, PackedMovesDecoder, PositionKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    pub m: PackedUciMove,
    pub turn: Color,
}

/// Replays the stored moves of a game (already truncated by
/// `pgn_to_parquet`) and finds the position with the given hash, as
/// `parquet_to_positions` would have recorded it (its last visit, if it
/// occurs repeatedly). The final position is never a visit. Stops replaying
/// at the first irreversible move after a visit.
pub fn find_visit(game: &BorrowedLichessGame<'_>, target: &PositionKey) -> Option<Visit> {
    let mut pos = game.initial_position();
    let variant = pos.variant();

    struct Find<'a> {
        decoder: PackedMovesDecoder<'a>,
        target: PositionKey,
        variant: shakmaty::variant::Variant,
    }

    impl WithPositionMut for Find<'_> {
        type Output = Option<Visit>;

        fn call<P: Position>(mut self, pos: &mut P) -> Option<Visit> {
            let mut hash: Zobrist128 = pos.zobrist_hash(EnPassantMode::Legal);
            let mut found = None;
            while let Some(m) = self.decoder.decode_next(pos).expect("valid moves") {
                if PositionKey::from_zobrist(hash, self.variant) == self.target {
                    found = Some(Visit {
                        m: PackedUciMove::pack(m.to_uci(CastlingMode::Chess960)),
                        turn: pos.turn(),
                    });
                }
                if found.is_some() && pos.is_irreversible(m) {
                    // The position cannot occur again, so that was its
                    // last visit.
                    break;
                }
                let updated = pos.update_zobrist_hash(hash, m, EnPassantMode::Legal);
                pos.play_unchecked(m);
                hash = updated.unwrap_or_else(|| pos.zobrist_hash(EnPassantMode::Legal));
            }
            found
        }
    }

    let decoder = PackedMovesDecoder::new(game.moves).expect("valid moves");
    pos.with_position_mut(Find {
        decoder,
        target: *target,
        variant,
    })
}

#[cfg(test)]
mod tests {
    use shakmaty::{
        ByColor, Chess, KnownOutcome, san::San, uci::UciMove, variant::VariantPosition,
    };

    use super::*;
    use crate::packed::{BorrowedGamePlayer, LaxDate, PackedMoves, RatingMode};

    fn game(sans: &str) -> PackedMoves {
        let mut pos = VariantPosition::Chess(Chess::default());
        let mut encoder = PackedMoves::encoder();
        for san in sans.split_whitespace() {
            let m = san.parse::<San>().unwrap().to_move(&pos).unwrap();
            encoder.encode_next(&pos, m);
            pos.play_unchecked(m);
        }
        encoder.finish()
    }

    fn after(sans: &str) -> VariantPosition {
        let mut pos = VariantPosition::Chess(Chess::default());
        for san in sans.split_whitespace() {
            let m = san.parse::<San>().unwrap().to_move(&pos).unwrap();
            pos.play_unchecked(m);
        }
        pos
    }

    #[test]
    fn test_find_visit() {
        let visit = |sans: &str, pos: &VariantPosition| {
            let moves = game(sans);
            let game = BorrowedLichessGame {
                players: ByColor {
                    white: BorrowedGamePlayer {
                        title: None,
                        name: "w",
                        rating: None,
                    },
                    black: BorrowedGamePlayer {
                        title: None,
                        name: "b",
                        rating: None,
                    },
                },
                date: LaxDate::from_ymd(2020, None, None).unwrap(),
                outcome: KnownOutcome::Draw,
                speed: None,
                rating_mode: RatingMode::Rated,
                setup: None,
                event: None,
                moves: &moves.0,
            };
            find_visit(&game, &PositionKey::new(pos)).map(|visit| (visit.m.unpack(), visit.turn))
        };
        let full = "Nf3 Nf6 Ng1 Ng8 e4 e5";
        let uci = |s: &str| s.parse::<UciMove>().unwrap();
        // The start position keeps the move eventually played.
        assert_eq!(visit(full, &after("")), Some((uci("e2e4"), Color::White)));
        // Unless the game was truncated before.
        assert_eq!(
            visit("Nf3 Nf6", &after("")),
            Some((uci("g1f3"), Color::White))
        );
        assert_eq!(
            visit(full, &after("Nf3")),
            Some((uci("g8f6"), Color::Black))
        );
        assert_eq!(visit(full, &after("e4")), Some((uci("e7e5"), Color::Black)));
        // The final position is not a visit.
        assert_eq!(visit(full, &after("e4 e5")), None);
        assert_eq!(visit("Nf3 Nf6 Ng1 Ng8 e4", &after("e4")), None);
        assert_eq!(visit(full, &after("d4")), None);
    }
}
