use serde::Deserialize;

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
