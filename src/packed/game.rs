use std::{fmt, str::FromStr};

use arbitrary::{Arbitrary, Unstructured};
use bitflags::bitflags;
use btoi::btou;
use bytes::{Buf, BufMut};
use serde::Serialize;
use serde_with::{TryFromInto, serde_as};
use shakmaty::{
    ByColor, CastlingMode, Color, EnPassantMode, KnownOutcome, Position, PositionError,
    packed::{PackedSetup, PackedUciMove},
    variant::{Variant, VariantPosition},
};
use thiserror::Error;

use crate::packed::{
    GameId, LaxDate, PackedMoves, Rating, RatingGroup, pack_bytes, pack_string, unpack_bytes,
    unpack_string,
};

pub fn unpack_uci_move<B: Buf>(buf: &mut B) -> PackedUciMove {
    let mut bytes = [0; PackedUciMove::BYTES];
    buf.copy_to_slice(&mut bytes);
    PackedUciMove::from_bytes(bytes)
}

pub fn outcome_bits(outcome: KnownOutcome) -> u8 {
    match outcome {
        KnownOutcome::Decisive {
            winner: Color::White,
        } => 1,
        KnownOutcome::Decisive {
            winner: Color::Black,
        } => 2,
        KnownOutcome::Draw => 3,
    }
}

pub fn outcome_from_bits(bits: u8) -> Option<KnownOutcome> {
    Some(match bits {
        1 => KnownOutcome::Decisive {
            winner: Color::White,
        },
        2 => KnownOutcome::Decisive {
            winner: Color::Black,
        },
        3 => KnownOutcome::Draw,
        _ => return None,
    })
}

/// Event tags of a game from an external collection (like the masters
/// database).
#[derive(Arbitrary, Debug, Clone, PartialEq, Eq)]
pub struct EventInfo {
    pub event: String,
    pub site: String,
    pub round: String,
}

impl EventInfo {
    pub fn as_borrowed(&self) -> BorrowedEventInfo<'_> {
        BorrowedEventInfo {
            event: &self.event,
            site: &self.site,
            round: &self.round,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct BorrowedEventInfo<'a> {
    pub event: &'a str,
    pub site: &'a str,
    pub round: &'a str,
}

impl<'a> BorrowedEventInfo<'a> {
    fn pack<B: BufMut>(&self, buf: &mut B) {
        pack_string(buf, self.event);
        pack_string(buf, self.site);
        pack_string(buf, self.round);
    }

    fn unpack(buf: &mut &'a [u8]) -> Self {
        BorrowedEventInfo {
            event: unpack_string(buf),
            site: unpack_string(buf),
            round: unpack_string(buf),
        }
    }

    pub fn into_owned(self) -> EventInfo {
        EventInfo {
            event: self.event.to_owned(),
            site: self.site.to_owned(),
            round: self.round.to_owned(),
        }
    }
}

#[serde_as]
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct GamePlayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub name: String,
    #[serde_as(as = "Option<TryFromInto<i32>>")]
    pub rating: Option<Rating>,
}

impl GamePlayer {
    pub fn as_borrowed(&self) -> BorrowedGamePlayer<'_> {
        BorrowedGamePlayer {
            title: self.title.as_deref(),
            name: &self.name,
            rating: self.rating,
        }
    }
}

impl Arbitrary<'_> for GamePlayer {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        Ok(GamePlayer {
            title: Some(u.arbitrary::<String>()?).filter(|s| !s.is_empty()),
            name: u.arbitrary()?,
            rating: u.arbitrary()?,
        })
    }
}

#[derive(Arbitrary, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Speed {
    UltraBullet = 1,
    Bullet = 2,
    Blitz = 3,
    Rapid = 4,
    Classical = 5,
    Correspondence = 6,
}

impl Speed {
    pub fn bits(self) -> u8 {
        match self {
            Speed::UltraBullet => 1,
            Speed::Bullet => 2,
            Speed::Blitz => 3,
            Speed::Rapid => 4,
            Speed::Classical => 5,
            Speed::Correspondence => 6,
        }
    }

    pub fn from_bits(bits: u8) -> Option<Speed> {
        Some(match bits {
            1 => Speed::UltraBullet,
            2 => Speed::Bullet,
            3 => Speed::Blitz,
            4 => Speed::Rapid,
            5 => Speed::Classical,
            6 => Speed::Correspondence,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Speed::UltraBullet => "ultraBullet",
            Speed::Bullet => "bullet",
            Speed::Blitz => "blitz",
            Speed::Rapid => "rapid",
            Speed::Classical => "classical",
            Speed::Correspondence => "correspondence",
        }
    }

    pub fn from_ascii(bytes: &[u8]) -> Result<Speed, InvalidSpeed> {
        Ok(match bytes {
            b"ultraBullet" => Speed::UltraBullet,
            b"bullet" => Speed::Bullet,
            b"blitz" => Speed::Blitz,
            b"rapid" => Speed::Rapid,
            b"classical" => Speed::Classical,
            b"correspondence" => Speed::Correspondence,
            _ => return Err(InvalidSpeed),
        })
    }

    fn from_seconds_and_increment(seconds: u32, increment: u32) -> Speed {
        let total = seconds.saturating_add(increment.saturating_mul(40));

        if total < 30 {
            Speed::UltraBullet
        } else if total < 180 {
            Speed::Bullet
        } else if total < 480 {
            Speed::Blitz
        } else if total < 1500 {
            Speed::Rapid
        } else if total < 21_600 {
            Speed::Classical
        } else {
            Speed::Correspondence
        }
    }

    pub fn from_time_control_tag(value: &[u8]) -> Result<Speed, InvalidSpeed> {
        if value == b"-" {
            return Ok(Speed::Correspondence);
        }

        let mut parts = value.splitn(2, |&ch| ch == b'+');
        let seconds = btou(parts.next().expect("first part")).map_err(|_| InvalidSpeed)?;
        let increment = btou(parts.next().ok_or(InvalidSpeed)?).map_err(|_| InvalidSpeed)?;
        Ok(Speed::from_seconds_and_increment(seconds, increment))
    }
}

bitflags! {
    /// A set of speeds.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct Speeds: u8 {
        const ULTRA_BULLET = 1 << 0;
        const BULLET = 1 << 1;
        const BLITZ = 1 << 2;
        const RAPID = 1 << 3;
        const CLASSICAL = 1 << 4;
        const CORRESPONDENCE = 1 << 5;
    }
}

impl From<Speed> for Speeds {
    fn from(speed: Speed) -> Speeds {
        match speed {
            Speed::UltraBullet => Speeds::ULTRA_BULLET,
            Speed::Bullet => Speeds::BULLET,
            Speed::Blitz => Speeds::BLITZ,
            Speed::Rapid => Speeds::RAPID,
            Speed::Classical => Speeds::CLASSICAL,
            Speed::Correspondence => Speeds::CORRESPONDENCE,
        }
    }
}

impl FromIterator<Speed> for Speeds {
    fn from_iter<I: IntoIterator<Item = Speed>>(iter: I) -> Speeds {
        iter.into_iter().map(Speeds::from).collect()
    }
}

impl fmt::Display for Speed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
#[error("Invalid speed")]
pub struct InvalidSpeed;

impl FromStr for Speed {
    type Err = InvalidSpeed;

    fn from_str(s: &str) -> Result<Speed, InvalidSpeed> {
        Speed::from_ascii(s.as_bytes())
    }
}

#[derive(Arbitrary, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum RatingMode {
    Casual,
    Rated,
}

impl RatingMode {
    pub fn bits(self) -> u8 {
        match self {
            RatingMode::Casual => 0,
            RatingMode::Rated => 1,
        }
    }

    pub fn from_bits(bits: u8) -> Option<RatingMode> {
        Some(match bits {
            0 => RatingMode::Casual,
            1 => RatingMode::Rated,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            RatingMode::Casual => "casual",
            RatingMode::Rated => "rated",
        }
    }

    pub fn from_ascii(bytes: &[u8]) -> Result<RatingMode, InvalidRatingMode> {
        Ok(match bytes {
            b"casual" => RatingMode::Casual,
            b"rated" => RatingMode::Rated,
            _ => return Err(InvalidRatingMode),
        })
    }
}

impl fmt::Display for RatingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
#[error("invalid rating mode")]
pub struct InvalidRatingMode;

impl FromStr for RatingMode {
    type Err = InvalidRatingMode;

    fn from_str(s: &str) -> Result<RatingMode, InvalidRatingMode> {
        RatingMode::from_ascii(s.as_bytes())
    }
}

bitflags! {
    /// A set of rating modes.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct RatingModes: u8 {
        const CASUAL = 1 << 0;
        const RATED = 1 << 1;
    }
}

impl From<RatingMode> for RatingModes {
    fn from(mode: RatingMode) -> RatingModes {
        match mode {
            RatingMode::Casual => RatingModes::CASUAL,
            RatingMode::Rated => RatingModes::RATED,
        }
    }
}

impl FromIterator<RatingMode> for RatingModes {
    fn from_iter<I: IntoIterator<Item = RatingMode>>(iter: I) -> RatingModes {
        iter.into_iter().map(RatingModes::from).collect()
    }
}

/// Packs the speed into the low 3 bits (0 if missing), the rating mode
/// into the next bit, and whether event tags follow into the bit after
/// that.
pub fn pack_game_flags<B: BufMut>(
    buf: &mut B,
    speed: Option<Speed>,
    rating_mode: RatingMode,
    has_event: bool,
) {
    buf.put_u8(
        speed.map_or(0, Speed::bits) | (rating_mode.bits() << 3) | (u8::from(has_event) << 4),
    );
}

pub fn unpack_game_flags<B: Buf>(buf: &mut B) -> (Option<Speed>, RatingMode, bool) {
    let packed = buf.get_u8();
    let speed = match packed & 0b111 {
        0 => None,
        bits => Some(Speed::from_bits(bits).expect("valid speed")),
    };
    let rating_mode = RatingMode::from_bits((packed >> 3) & 1).expect("valid rating mode");
    let has_event = (packed >> 4) & 1 != 0;
    (speed, rating_mode, has_event)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LichessGame {
    pub players: ByColor<GamePlayer>,
    pub date: LaxDate,
    pub outcome: KnownOutcome,
    pub speed: Option<Speed>,
    pub rating_mode: RatingMode,
    pub setup: Option<PackedSetup>,
    pub event: Option<EventInfo>,
    pub moves: PackedMoves,
}

impl LichessGame {
    pub fn as_borrowed(&self) -> BorrowedLichessGame<'_> {
        BorrowedLichessGame {
            players: self.players.as_ref().map(GamePlayer::as_borrowed),
            date: self.date,
            outcome: self.outcome,
            speed: self.speed,
            rating_mode: self.rating_mode,
            setup: self.setup.as_ref().map(PackedSetup::as_bytes),
            event: self.event.as_ref().map(EventInfo::as_borrowed),
            moves: &self.moves.0,
        }
    }

    pub fn initial_position(&self) -> Result<VariantPosition, InvalidSetup> {
        initial_position(self.setup.as_ref())
    }

    pub fn pack_entire<B: BufMut>(&self, buf: &mut B) {
        self.as_borrowed().pack_entire(buf);
    }

    pub fn unpack_entire(bytes: &[u8]) -> Self {
        BorrowedLichessGame::unpack_entire(bytes).into_owned()
    }
}

impl Arbitrary<'_> for LichessGame {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        let setup = Option::<VariantPosition>::arbitrary(u)?;

        let mut packed_moves = PackedMoves::encoder();
        let mut pos = setup
            .clone()
            .unwrap_or_else(|| VariantPosition::new(Variant::Chess));
        for _ in 0..u.arbitrary_len::<usize>()? {
            let legals = pos.legal_moves();
            let Ok(&m) = u.choose(&legals) else { break };
            packed_moves.encode_next(&pos, m);
            pos.play_unchecked(m);
        }

        Ok(LichessGame {
            players: u.arbitrary()?,
            date: u.arbitrary()?,
            outcome: u.arbitrary()?,
            speed: u.arbitrary()?,
            rating_mode: u.arbitrary()?,
            setup: setup.map(|s| {
                PackedSetup::pack_variant_normalized(&s.to_setup(EnPassantMode::Legal), s.variant())
                    .expect("packed setup")
            }),
            event: u.arbitrary()?,
            moves: packed_moves.finish(),
        })
    }
}

pub const BOT_TITLE: &str = "BOT";

/// A player of a `BorrowedLichessGame`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct BorrowedGamePlayer<'a> {
    pub title: Option<&'a str>,
    pub name: &'a str,
    pub rating: Option<Rating>,
}

impl<'a> BorrowedGamePlayer<'a> {
    fn pack<B: BufMut>(&self, buf: &mut B) {
        pack_string(buf, self.title.unwrap_or(""));
        pack_string(buf, self.name);
        Rating::pack_optional(self.rating, buf);
    }

    fn unpack(buf: &mut &'a [u8]) -> Self {
        BorrowedGamePlayer {
            title: Some(unpack_string(buf)).filter(|title| !title.is_empty()),
            name: unpack_string(buf),
            rating: Rating::unpack_optional(buf),
        }
    }

    pub fn into_owned(self) -> GamePlayer {
        GamePlayer {
            title: self.title.map(str::to_owned),
            name: self.name.to_owned(),
            rating: self.rating,
        }
    }
}

/// A `LichessGame` borrowing its data, with setup and moves not yet decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowedLichessGame<'a> {
    pub players: ByColor<BorrowedGamePlayer<'a>>,
    pub date: LaxDate,
    pub outcome: KnownOutcome,
    pub speed: Option<Speed>,
    pub rating_mode: RatingMode,
    /// `PackedSetup`, decoded by `initial_position()`.
    pub setup: Option<&'a [u8]>,
    pub event: Option<BorrowedEventInfo<'a>>,
    /// `PackedMoves`.
    pub moves: &'a [u8],
}

impl<'a> BorrowedLichessGame<'a> {
    pub fn unpack_entire(bytes: &'a [u8]) -> Self {
        let mut buf = bytes;
        let players = ByColor {
            white: BorrowedGamePlayer::unpack(&mut buf),
            black: BorrowedGamePlayer::unpack(&mut buf),
        };
        let date = LaxDate::unpack(&mut buf);
        let outcome = outcome_from_bits(buf.get_u8()).expect("valid outcome");
        let (speed, rating_mode, has_event) = unpack_game_flags(&mut buf);
        let setup = Some(unpack_bytes(&mut buf)).filter(|setup| !setup.is_empty());
        let event = has_event.then(|| BorrowedEventInfo::unpack(&mut buf));
        BorrowedLichessGame {
            players,
            date,
            outcome,
            speed,
            rating_mode,
            setup,
            event,
            moves: buf,
        }
    }

    pub fn into_owned(self) -> LichessGame {
        LichessGame {
            players: self.players.map(BorrowedGamePlayer::into_owned),
            date: self.date,
            outcome: self.outcome,
            speed: self.speed,
            rating_mode: self.rating_mode,
            setup: self
                .setup
                .map(|bytes| PackedSetup::try_from_bytes(bytes).expect("valid setup")),
            event: self.event.map(BorrowedEventInfo::into_owned),
            moves: PackedMoves(self.moves.to_vec()),
        }
    }

    pub fn initial_position(&self) -> Result<VariantPosition, InvalidSetup> {
        let setup = self
            .setup
            .map(|bytes| PackedSetup::try_from_bytes(bytes).expect("valid setup"));
        initial_position(setup.as_ref())
    }

    pub fn has_bot(&self) -> bool {
        self.players
            .iter()
            .any(|player| player.title == Some(BOT_TITLE))
    }

    pub fn ratings_or_default(&self) -> ByColor<Rating> {
        self.players.map(|player| player.rating.unwrap_or_default())
    }

    pub fn rating_group(&self) -> RatingGroup {
        RatingGroup::select_game(self.ratings_or_default())
    }

    pub fn pack_entire<B: BufMut>(&self, buf: &mut B) {
        self.players.white.pack(buf);
        self.players.black.pack(buf);
        self.date.pack(buf);
        buf.put_u8(outcome_bits(self.outcome));
        pack_game_flags(buf, self.speed, self.rating_mode, self.event.is_some());
        pack_bytes(buf, self.setup.unwrap_or(b""));
        if let Some(event) = self.event {
            event.pack(buf);
        }
        buf.put_slice(self.moves);
    }
}

#[derive(Error, Debug)]
#[error("setup no longer considered valid: {0}")]
pub struct InvalidSetup(Box<PositionError<VariantPosition>>);

pub fn initial_position(setup: Option<&PackedSetup>) -> Result<VariantPosition, InvalidSetup> {
    match setup {
        None => Ok(VariantPosition::new(Variant::Chess)),
        Some(setup) => {
            let (setup, variant) = setup.unpack_variant().expect("valid setup");
            VariantPosition::from_setup(variant, setup, CastlingMode::Chess960)
                .map_err(|err| InvalidSetup(Box::new(err)))
        }
    }
}

/// Packs a value of the games column family: the id of the game, then the
/// packed game.
pub fn pack_game_value<B: BufMut>(id: GameId, game: &BorrowedLichessGame<'_>, buf: &mut B) {
    id.pack(buf);
    game.pack_entire(buf);
}

/// Unpacks a value of the games column family.
pub fn unpack_game_value(mut bytes: &[u8]) -> (GameId, BorrowedLichessGame<'_>) {
    let id = GameId::unpack(&mut bytes);
    (id, BorrowedLichessGame::unpack_entire(bytes))
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;
    use shakmaty::Setup;

    use super::*;

    #[test]
    fn test_initial_position() {
        assert_eq!(
            initial_position(None).unwrap(),
            VariantPosition::new(Variant::Chess)
        );
        let empty = PackedSetup::pack_variant(&Setup::empty(), Variant::Chess).unwrap();
        assert!(initial_position(Some(&empty)).is_err());
    }

    #[test]
    fn test_speed_str_roundtrip() {
        for speed in [
            Speed::UltraBullet,
            Speed::Bullet,
            Speed::Blitz,
            Speed::Rapid,
            Speed::Classical,
            Speed::Correspondence,
        ] {
            assert_eq!(Speed::from_ascii(speed.as_str().as_bytes()).unwrap(), speed);
        }
    }

    #[test]
    fn test_rating_mode_str_roundtrip() {
        for rating_mode in [RatingMode::Casual, RatingMode::Rated] {
            assert_eq!(
                RatingMode::from_ascii(rating_mode.as_str().as_bytes()).unwrap(),
                rating_mode
            );
        }
    }

    #[test]
    fn test_speed_from_time_control_tag() {
        assert_eq!(
            Speed::from_time_control_tag(b"-").unwrap(),
            Speed::Correspondence
        );
        assert_eq!(
            Speed::from_time_control_tag(b"60+0").unwrap(),
            Speed::Bullet
        );
        assert_eq!(
            Speed::from_time_control_tag(b"120+2").unwrap(),
            Speed::Blitz
        );
    }

    #[test]
    fn test_pack_roundtrip() {
        arbtest(|u| {
            let game: LichessGame = u.arbitrary()?;

            let mut buf = Vec::new();
            game.pack_entire(&mut buf);
            assert_eq!(BorrowedLichessGame::unpack_entire(&buf), game.as_borrowed());
            assert_eq!(LichessGame::unpack_entire(&buf), game);
            Ok(())
        });
    }
}
