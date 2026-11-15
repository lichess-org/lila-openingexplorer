use arbitrary::{Arbitrary, Unstructured};
use bytes::{Buf, BufMut};
use shakmaty::{
    Color, EnPassantMode, KnownOutcome, Position,
    packed::PackedSetup,
    variant::{Variant, VariantPosition},
};

use crate::packed::{
    GameId, Month, PackedMoves, Rating, RatingMode, Speed, color_bits, color_from_bits,
    initial_position, outcome_bits, outcome_from_bits, pack_bytes, pack_string, unpack_bytes,
    unpack_string,
};

/// A game of a player.
///
/// ```text
/// record := GameId flags Month rating(player) rating(opponent)
///           string(opponent name) string(opponent title) string(player title)
///           bytes(PackedSetup, empty for the standard initial position)
///           PackedMoves (rest)
/// flags  := pack_player_game_flags
/// rating := Rating::pack_fixed
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerGame {
    pub id: GameId,
    pub speed: Speed,
    pub mode: RatingMode,
    /// Month of the last move.
    pub month: Month,
    pub outcome: KnownOutcome,
    /// The side of the indexed player.
    pub color: Color,
    pub player_rating: Rating,
    pub opponent_rating: Rating,
    pub opponent_name: String,
    pub opponent_title: Option<String>,
    pub player_title: Option<String>,
    pub setup: Option<PackedSetup>,
    /// Only up to the indexed ply.
    pub moves: PackedMoves,
}

impl PlayerGame {
    pub fn as_borrowed(&self) -> BorrowedPlayerGame<'_> {
        BorrowedPlayerGame {
            id: self.id,
            speed: self.speed,
            mode: self.mode,
            month: self.month,
            outcome: self.outcome,
            color: self.color,
            player_rating: self.player_rating,
            opponent_rating: self.opponent_rating,
            opponent_name: &self.opponent_name,
            opponent_title: self.opponent_title.as_deref(),
            player_title: self.player_title.as_deref(),
            setup: self.setup.as_ref().map(PackedSetup::as_bytes),
            moves: &self.moves.0,
        }
    }

    pub fn pack(&self, buf: &mut Vec<u8>) {
        self.as_borrowed().pack(buf);
    }

    pub fn setup_of(pos: &VariantPosition) -> Option<PackedSetup> {
        if *pos == VariantPosition::new(Variant::Chess) {
            None
        } else {
            Some(
                PackedSetup::pack_variant_normalized(
                    &pos.to_setup(EnPassantMode::Legal),
                    pos.variant(),
                )
                .expect("packable setup"),
            )
        }
    }
}

impl Arbitrary<'_> for PlayerGame {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<PlayerGame> {
        let initial = Option::<VariantPosition>::arbitrary(u)?
            .unwrap_or_else(|| VariantPosition::new(Variant::Chess));
        let mut moves = PackedMoves::encoder();
        let mut pos = initial.clone();
        for _ in 0..u.arbitrary_len::<usize>()? {
            let legals = pos.legal_moves();
            let Ok(&m) = u.choose(&legals) else { break };
            moves.encode_next(&pos, m);
            pos.play_unchecked(m);
        }
        let title = |u: &mut Unstructured<'_>| {
            u.arbitrary::<Option<String>>()
                .map(|title| title.filter(|t| !t.is_empty()))
        };
        Ok(PlayerGame {
            id: u.arbitrary()?,
            speed: u.arbitrary()?,
            mode: u.arbitrary()?,
            month: u.arbitrary()?,
            outcome: u.arbitrary()?,
            color: u.arbitrary()?,
            player_rating: u.arbitrary()?,
            opponent_rating: u.arbitrary()?,
            opponent_name: u.arbitrary()?,
            opponent_title: title(u)?,
            player_title: title(u)?,
            setup: PlayerGame::setup_of(&initial),
            moves: moves.finish(),
        })
    }
}

pub fn pack_player_game_flags<B: BufMut>(
    buf: &mut B,
    speed: Speed,
    mode: RatingMode,
    color: Color,
    outcome: KnownOutcome,
) {
    buf.put_u8(
        speed.bits() | (mode.bits() << 3) | (color_bits(color) << 4) | (outcome_bits(outcome) << 5),
    );
}

pub fn unpack_player_game_flags<B: Buf>(buf: &mut B) -> (Speed, RatingMode, Color, KnownOutcome) {
    let packed = buf.get_u8();
    (
        Speed::from_bits(packed & 0b111).expect("valid speed"),
        RatingMode::from_bits((packed >> 3) & 1).expect("valid rating mode"),
        color_from_bits((packed >> 4) & 1).expect("valid color"),
        outcome_from_bits(packed >> 5).expect("valid outcome"),
    )
}

/// A `PlayerGame` borrowing its data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorrowedPlayerGame<'a> {
    pub id: GameId,
    pub speed: Speed,
    pub mode: RatingMode,
    pub month: Month,
    pub outcome: KnownOutcome,
    pub color: Color,
    pub player_rating: Rating,
    pub opponent_rating: Rating,
    pub opponent_name: &'a str,
    pub opponent_title: Option<&'a str>,
    pub player_title: Option<&'a str>,
    /// `PackedSetup`, decoded by `initial_position()`.
    pub setup: Option<&'a [u8]>,
    /// `PackedMoves`.
    pub moves: &'a [u8],
}

impl<'a> BorrowedPlayerGame<'a> {
    pub fn pack(&self, buf: &mut Vec<u8>) {
        self.id.pack(buf);
        pack_player_game_flags(buf, self.speed, self.mode, self.color, self.outcome);
        self.month.pack(buf);
        self.player_rating.pack_fixed(buf);
        self.opponent_rating.pack_fixed(buf);
        pack_string(buf, self.opponent_name);
        pack_string(buf, self.opponent_title.unwrap_or(""));
        pack_string(buf, self.player_title.unwrap_or(""));
        pack_bytes(buf, self.setup.unwrap_or(b""));
        buf.put_slice(self.moves);
    }

    pub fn unpack(bytes: &'a [u8]) -> BorrowedPlayerGame<'a> {
        let mut buf = bytes;
        let id = GameId::unpack(&mut buf);
        let (speed, mode, color, outcome) = unpack_player_game_flags(&mut buf);
        let month = Month::unpack(&mut buf);
        let player_rating = Rating::unpack_fixed(&mut buf);
        let opponent_rating = Rating::unpack_fixed(&mut buf);
        let opponent_name = unpack_string(&mut buf);
        let opponent_title = Some(unpack_string(&mut buf)).filter(|t| !t.is_empty());
        let player_title = Some(unpack_string(&mut buf)).filter(|t| !t.is_empty());
        let setup = Some(unpack_bytes(&mut buf)).filter(|s| !s.is_empty());
        BorrowedPlayerGame {
            id,
            speed,
            mode,
            color,
            outcome,
            month,
            player_rating,
            opponent_rating,
            opponent_name,
            opponent_title,
            player_title,
            setup,
            moves: buf,
        }
    }

    pub fn initial_position(&self) -> VariantPosition {
        let setup = self
            .setup
            .map(|bytes| PackedSetup::try_from_bytes(bytes).expect("valid setup"));
        initial_position(setup.as_ref())
    }

    pub fn into_owned(self) -> PlayerGame {
        PlayerGame {
            id: self.id,
            speed: self.speed,
            mode: self.mode,
            month: self.month,
            outcome: self.outcome,
            color: self.color,
            player_rating: self.player_rating,
            opponent_rating: self.opponent_rating,
            opponent_name: self.opponent_name.to_owned(),
            opponent_title: self.opponent_title.map(str::to_owned),
            player_title: self.player_title.map(str::to_owned),
            setup: self
                .setup
                .map(|bytes| PackedSetup::try_from_bytes(bytes).expect("valid setup")),
            moves: PackedMoves(self.moves.to_vec()),
        }
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_roundtrip() {
        arbtest(|u| {
            let game: PlayerGame = u.arbitrary()?;
            let mut buf = Vec::new();
            game.pack(&mut buf);
            assert_eq!(BorrowedPlayerGame::unpack(&buf).into_owned(), game);
            Ok(())
        });
    }

    #[test]
    fn test_setup_of() {
        assert_eq!(
            PlayerGame::setup_of(&VariantPosition::new(Variant::Chess)),
            None
        );
        let atomic = VariantPosition::new(Variant::Atomic);
        let setup = PlayerGame::setup_of(&atomic).unwrap();
        assert_eq!(initial_position(Some(&setup)), atomic);
    }
}
