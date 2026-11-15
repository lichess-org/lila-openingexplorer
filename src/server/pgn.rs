use std::fmt::Write;

use shakmaty::{
    Color, EnPassantMode, Position,
    fen::Fen,
    san::SanPlus,
    variant::{Variant, VariantPosition},
};

use crate::packed::{BorrowedLichessGame, GameId, PackedMovesDecoder};

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Renders a masters game as PGN.
pub fn masters_pgn(id: GameId, game: &BorrowedLichessGame<'_>) -> String {
    let mut pgn = String::new();
    let event = game.event.as_ref();
    let _ = writeln!(
        pgn,
        "[Event \"{}\"]",
        escape(event.map_or("?", |event| event.event))
    );
    let _ = writeln!(
        pgn,
        "[Site \"{}\"]",
        escape(event.map_or("?", |event| event.site))
    );
    let _ = writeln!(pgn, "[Date \"{}\"]", game.date.pgn());
    let _ = writeln!(
        pgn,
        "[Round \"{}\"]",
        escape(event.map_or("?", |event| event.round))
    );
    let _ = writeln!(pgn, "[White \"{}\"]", escape(game.players.white.name));
    let _ = writeln!(pgn, "[Black \"{}\"]", escape(game.players.black.name));
    let _ = writeln!(pgn, "[Result \"{}\"]", game.outcome.as_str());
    for (tag, player) in [
        ("WhiteElo", game.players.white),
        ("BlackElo", game.players.black),
    ] {
        let _ = writeln!(
            pgn,
            "[{tag} \"{}\"]",
            player
                .rating
                .map_or("?".to_owned(), |r| i32::from(r).to_string())
        );
    }
    let _ = writeln!(pgn, "[LichessId \"{id}\"]");

    let mut pos = game.initial_position();
    if pos.variant() != Variant::Chess || pos != VariantPosition::new(Variant::Chess) {
        if pos.variant() != Variant::Chess {
            let _ = writeln!(pgn, "[Variant \"{}\"]", pos.variant());
        }
        let _ = writeln!(
            pgn,
            "[FEN \"{}\"]",
            Fen::from_position(&pos, EnPassantMode::Legal)
        );
        let _ = writeln!(pgn, "[SetUp \"1\"]");
    }
    pgn.push('\n');

    let mut decoder = PackedMovesDecoder::new(game.moves).expect("valid moves");
    let mut first = true;
    while let Some(m) = decoder.decode_next(&pos).expect("valid moves") {
        if pos.turn() == Color::White {
            let _ = write!(pgn, "{}. ", pos.fullmoves());
        } else if first {
            let _ = write!(pgn, "{}... ", pos.fullmoves());
        }
        let _ = write!(
            pgn,
            "{} ",
            SanPlus::from_move_and_play_unchecked(&mut pos, m)
        );
        first = false;
    }
    let _ = writeln!(pgn, "{}", game.outcome.as_str());
    pgn
}

#[cfg(test)]
mod tests {
    use shakmaty::{ByColor, Chess, KnownOutcome, san::San};

    use super::*;
    use crate::packed::{
        BorrowedEventInfo, BorrowedGamePlayer, LaxDate, PackedMoves, Rating, RatingMode,
    };

    #[test]
    fn test_masters_pgn() {
        let mut pos = VariantPosition::Chess(Chess::default());
        let mut encoder = PackedMoves::encoder();
        for san in "e4 e5 Nf3".split_whitespace() {
            let m = san.parse::<San>().unwrap().to_move(&pos).unwrap();
            encoder.encode_next(&pos, m);
            pos.play_unchecked(m);
        }
        let moves = encoder.finish();
        let game = BorrowedLichessGame {
            players: ByColor {
                white: BorrowedGamePlayer {
                    title: None,
                    name: "Carlsen, Magnus",
                    rating: Some(Rating::try_from(2837).unwrap()),
                },
                black: BorrowedGamePlayer {
                    title: None,
                    name: "Chadaev, Nikolay",
                    rating: Some(Rating::try_from(2580).unwrap()),
                },
            },
            date: LaxDate::from_ymd(2012, Some(7), None).unwrap(),
            outcome: KnownOutcome::Decisive {
                winner: Color::White,
            },
            speed: None,
            rating_mode: RatingMode::Rated,
            setup: None,
            event: Some(BorrowedEventInfo {
                event: "Wch Blitz",
                site: "Astana",
                round: "23",
            }),
            moves: &moves.0,
        };
        let pgn = masters_pgn("aAbqI4ey".parse().unwrap(), &game);
        assert_eq!(
            pgn,
            "[Event \"Wch Blitz\"]\n[Site \"Astana\"]\n[Date \"2012.07.??\"]\n[Round \"23\"]\n[White \"Carlsen, Magnus\"]\n[Black \"Chadaev, Nikolay\"]\n[Result \"1-0\"]\n[WhiteElo \"2837\"]\n[BlackElo \"2580\"]\n[LichessId \"aAbqI4ey\"]\n\n1. e4 e5 2. Nf3 1-0\n"
        );
    }
}
