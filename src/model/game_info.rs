use crate::{
    api::ColorProxy,
    model::{Speed, UserName},
};
use serde::Serialize;
use serde_with::{serde_as, DisplayFromStr, FromInto};
use shakmaty::Color;

#[serde_as]
#[derive(Serialize)]
pub struct GameInfo {
    #[serde_as(as = "Option<FromInto<ColorProxy>>")]
    winner: Option<Color>,
    speed: Speed,
    rated: bool,
    white: Player,
    black: Player,
    year: u32,
}

#[serde_as]
#[derive(Serialize)]
struct Player {
    #[serde_as(as = "DisplayFromStr")]
    name: UserName,
    rating: u16,
}
