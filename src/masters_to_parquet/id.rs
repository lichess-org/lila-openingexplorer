//! Deterministic masters game ids: the first 8 characters of the base64
//! encoded sha256 digest (with `a` and `b` instead of `+` and `/`) of the
//! JSON representation of the game, as produced by Python's `json.dumps`
//! with `sort_keys=True`.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};
use shakmaty::{ByColor, CastlingMode, Color, File, KnownOutcome, Move, uci::UciMove};

use crate::packed::GameId;

pub struct IdInput<'a> {
    pub event: &'a str,
    pub site: &'a str,
    /// Date as in the PGN tag, like `1994.??.??`.
    pub date: &'a str,
    pub round: &'a str,
    pub players: ByColor<(&'a str, i32)>,
    pub outcome: KnownOutcome,
    /// Space separated UCI moves, see [`push_uci`].
    pub moves: &'a str,
}

impl IdInput<'_> {
    pub fn game_id(&self) -> GameId {
        let mut json = String::with_capacity(512 + self.moves.len());
        json.push_str("{\"black\": ");
        push_player(&mut json, self.players.black);
        json.push_str(", \"date\": ");
        push_str(&mut json, self.date);
        json.push_str(", \"event\": ");
        push_str(&mut json, self.event);
        json.push_str(", \"moves\": ");
        push_str(&mut json, self.moves);
        json.push_str(", \"round\": ");
        push_str(&mut json, self.round);
        json.push_str(", \"site\": ");
        push_str(&mut json, self.site);
        json.push_str(", \"white\": ");
        push_player(&mut json, self.players.white);
        json.push_str(", \"winner\": ");
        json.push_str(match self.outcome {
            KnownOutcome::Decisive {
                winner: Color::White,
            } => "\"white\"",
            KnownOutcome::Decisive {
                winner: Color::Black,
            } => "\"black\"",
            KnownOutcome::Draw => "null",
        });
        json.push('}');

        let digest = Sha256::digest(json.as_bytes());
        let mut id = [0u8; 8];
        for (i, chunk) in digest[..6].chunks(3).enumerate() {
            let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
            for j in 0..4 {
                id[i * 4 + j] = base64_char(((n >> (18 - 6 * j)) & 0x3f) as u8);
            }
        }
        GameId::from_ascii(&id).expect("base64 chars are base62 chars")
    }
}

/// Appends a move in UCI notation, like python-chess does for a board
/// that is not in Chess960 mode: castling from the standard squares is
/// written as the king move (`e1g1`), any other castling as king to rook.
pub fn push_uci(moves: &mut String, m: Move) {
    if !moves.is_empty() {
        moves.push(' ');
    }
    let mode = match m {
        Move::Castle { king, rook }
            if king.file() == File::E && matches!(rook.file(), File::A | File::H) =>
        {
            CastlingMode::Standard
        }
        _ => CastlingMode::Chess960,
    };
    let _ = write!(moves, "{}", UciMove::from_move(m, mode));
}

fn base64_char(n: u8) -> u8 {
    match n {
        0..=25 => b'A' + n,
        26..=51 => b'a' + n - 26,
        52..=61 => b'0' + n - 52,
        62 => b'a',
        _ => b'b',
    }
}

fn push_player(json: &mut String, (name, rating): (&str, i32)) {
    json.push_str("{\"name\": ");
    push_str(json, name);
    let _ = write!(json, ", \"rating\": {rating}}}");
}

/// Appends a string literal like `json.dumps` with `ensure_ascii=True`.
fn push_str(json: &mut String, s: &str) {
    json.push('"');
    for c in s.chars() {
        match c {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            '\x08' => json.push_str("\\b"),
            '\x0c' => json.push_str("\\f"),
            ' '..='~' => json.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(json, "\\u{unit:04x}");
                }
            }
        }
    }
    json.push('"');
}

#[cfg(test)]
mod tests {
    use shakmaty::{Chess, Position, san::San};

    use super::*;

    #[test]
    fn test_game_id() {
        let mut moves = String::new();
        let mut pos = Chess::default();
        for san in ["e4", "e5", "Nf3", "Nc6", "Bc4", "Nf6", "O-O"] {
            let m = san.parse::<San>().unwrap().to_move(&pos).unwrap();
            push_uci(&mut moves, m);
            pos.play_unchecked(m);
        }
        assert_eq!(moves, "e2e4 e7e5 g1f3 b8c6 f1c4 g8f6 e1g1");

        let id = IdInput {
            event: "Ev \"q\" \\ é 😀\t",
            site: "Site",
            date: "2021.??.??",
            round: "1.1",
            players: ByColor {
                white: ("White, W", 2800),
                black: ("Black", 2700),
            },
            outcome: KnownOutcome::Decisive {
                winner: Color::White,
            },
            moves: &moves,
        }
        .game_id();

        // Computed with the Python reference implementation.
        assert_eq!(id.to_string(), "xYDtis4f");
    }
}
