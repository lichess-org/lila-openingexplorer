use serde::Deserialize;
use shakmaty::variant::Variant;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LilaVariant {
    Antichess,
    Atomic,
    Chess960,
    Crazyhouse,
    FromPosition,
    Horde,
    KingOfTheHill,
    RacingKings,
    Standard,
    ThreeCheck,
}

impl From<LilaVariant> for Variant {
    fn from(variant: LilaVariant) -> Variant {
        match variant {
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
