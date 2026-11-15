use shakmaty::variant::Variant;
use thiserror::Error;

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub enum LilaVariant {
    #[default]
    Standard,
    Chess960,
    FromPosition,
    Antichess,
    Atomic,
    Crazyhouse,
    Horde,
    KingOfTheHill,
    RacingKings,
    ThreeCheck,
}

#[derive(Debug, Error)]
#[error("unknown variant")]
pub struct UnknownLilaVariant;

impl LilaVariant {
    pub const ALL: [LilaVariant; 10] = [
        LilaVariant::Standard,
        LilaVariant::Chess960,
        LilaVariant::FromPosition,
        LilaVariant::Antichess,
        LilaVariant::Atomic,
        LilaVariant::Crazyhouse,
        LilaVariant::Horde,
        LilaVariant::KingOfTheHill,
        LilaVariant::RacingKings,
        LilaVariant::ThreeCheck,
    ];

    /// Parses a key of lila's API, like `kingOfTheHill`.
    pub fn from_key(key: &str) -> Result<LilaVariant, UnknownLilaVariant> {
        LilaVariant::ALL
            .into_iter()
            .find(|variant| variant.key() == key)
            .ok_or(UnknownLilaVariant)
    }

    pub fn key(self) -> &'static str {
        match self {
            LilaVariant::Standard => "standard",
            LilaVariant::Chess960 => "chess960",
            LilaVariant::FromPosition => "fromPosition",
            LilaVariant::Antichess => "antichess",
            LilaVariant::Atomic => "atomic",
            LilaVariant::Crazyhouse => "crazyhouse",
            LilaVariant::Horde => "horde",
            LilaVariant::KingOfTheHill => "kingOfTheHill",
            LilaVariant::RacingKings => "racingKings",
            LilaVariant::ThreeCheck => "threeCheck",
        }
    }

    pub fn from_pgn_tag(value: &[u8]) -> Result<LilaVariant, UnknownLilaVariant> {
        Ok(match value {
            b"Standard" => LilaVariant::Standard,
            b"Chess960" => LilaVariant::Chess960,
            b"From Position" => LilaVariant::FromPosition,
            b"Antichess" => LilaVariant::Antichess,
            b"Atomic" => LilaVariant::Atomic,
            b"Crazyhouse" => LilaVariant::Crazyhouse,
            b"Horde" => LilaVariant::Horde,
            b"King of the Hill" => LilaVariant::KingOfTheHill,
            b"Racing Kings" => LilaVariant::RacingKings,
            b"Three-check" => LilaVariant::ThreeCheck,
            _ => return Err(UnknownLilaVariant),
        })
    }

    pub fn pgn_tag(self) -> &'static str {
        match self {
            LilaVariant::Standard => "Standard",
            LilaVariant::Chess960 => "Chess960",
            LilaVariant::FromPosition => "From Position",
            LilaVariant::Antichess => "Antichess",
            LilaVariant::Atomic => "Atomic",
            LilaVariant::Crazyhouse => "Crazyhouse",
            LilaVariant::Horde => "Horde",
            LilaVariant::KingOfTheHill => "King of the Hill",
            LilaVariant::RacingKings => "Racing Kings",
            LilaVariant::ThreeCheck => "Three-check",
        }
    }

    pub fn variant(self) -> Variant {
        match self {
            LilaVariant::Standard | LilaVariant::Chess960 | LilaVariant::FromPosition => {
                Variant::Chess
            }
            LilaVariant::Antichess => Variant::Antichess,
            LilaVariant::Atomic => Variant::Atomic,
            LilaVariant::Crazyhouse => Variant::Crazyhouse,
            LilaVariant::Horde => Variant::Horde,
            LilaVariant::KingOfTheHill => Variant::KingOfTheHill,
            LilaVariant::RacingKings => Variant::RacingKings,
            LilaVariant::ThreeCheck => Variant::ThreeCheck,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_roundtrip() {
        for variant in LilaVariant::ALL {
            assert_eq!(LilaVariant::from_key(variant.key()).unwrap(), variant);
        }
        assert!(LilaVariant::from_key("Chess960").is_err());
    }

    #[test]
    fn test_pgn_tag_roundtrip() {
        for variant in LilaVariant::ALL {
            assert_eq!(
                LilaVariant::from_pgn_tag(variant.pgn_tag().as_bytes()).unwrap(),
                variant
            );
        }
    }
}
