use std::cmp::{max, min};

use serde::Deserialize;
use serde_with::{serde_as, CommaSeparator, DisplayFromStr, StringWithSeparator, TryFromInto};
use shakmaty::{
    fen::Fen,
    uci::Uci,
    variant::{Variant, VariantPosition},
    zobrist::Zobrist,
    CastlingMode, Color, PositionError,
};

use crate::{
    api::{Error, LaxFen, LilaVariant},
    model::{Mode, Month, RatingGroup, Speed, UserName, Year},
    opening::{Opening, Openings},
};

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct MastersQuery {
    #[serde(flatten)]
    pub play: Play,
    #[serde_as(as = "TryFromInto<u16>")]
    #[serde(default)]
    pub since: Year,
    #[serde_as(as = "TryFromInto<u16>")]
    #[serde(default = "Year::max_value")]
    pub until: Year,
    #[serde(flatten)]
    pub limits: Limits,
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct LichessQuery {
    #[serde(flatten)]
    pub play: Play,
    #[serde(flatten)]
    pub limits: Limits,
    #[serde(flatten)]
    pub filter: LichessQueryFilter,
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct LichessQueryFilter {
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, Speed>>")]
    #[serde(default)]
    pub speeds: Option<Vec<Speed>>,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, RatingGroup>>")]
    #[serde(default)]
    pub ratings: Option<Vec<RatingGroup>>,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default)]
    pub since: Month,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "Month::max_value")]
    pub until: Month,
}

impl LichessQueryFilter {
    pub fn contains_speed(&self, speed: Speed) -> bool {
        self.speeds
            .as_ref()
            .map_or(true, |speeds| speeds.contains(&speed))
    }

    pub fn contains_rating_group(&self, rating_group: RatingGroup) -> bool {
        self.ratings.as_ref().map_or(true, |ratings| {
            ratings.contains(&max(
                RatingGroup::Group1600,
                min(rating_group, RatingGroup::Group2500),
            ))
        })
    }

    pub fn top_group(&self) -> Option<RatingGroup> {
        let mut top_group = None;
        for group in RatingGroup::ALL.into_iter().rev() {
            if !self.contains_rating_group(group) || group < RatingGroup::Group2000 {
                break;
            } else {
                top_group = Some(group);
            }
        }
        top_group
    }
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct PlayerQuery {
    #[serde(flatten)]
    pub play: Play,
    #[serde_as(as = "DisplayFromStr")]
    pub player: UserName,
    #[serde_as(as = "DisplayFromStr")]
    pub color: Color,
    #[serde(flatten)]
    pub filter: PlayerQueryFilter,
    #[serde(flatten)]
    pub limits: Limits,
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct PlayerQueryFilter {
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, Mode>>")]
    #[serde(default)]
    pub modes: Option<Vec<Mode>>,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, Speed>>")]
    #[serde(default)]
    pub speeds: Option<Vec<Speed>>,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default)]
    pub since: Month,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "Month::max_value")]
    pub until: Month,
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct Play {
    #[serde(default)]
    pub variant: LilaVariant,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub fen: Option<LaxFen>,
    #[serde_as(as = "StringWithSeparator<CommaSeparator, Uci>")]
    #[serde(default)]
    pub play: Vec<Uci>,
}

impl Play {
    pub fn position(
        self,
        openings: &Openings,
    ) -> Result<(Variant, Zobrist<VariantPosition, u128>, Option<&Opening>), Error> {
        let variant = Variant::from(self.variant);
        let mut pos = Zobrist::new(match self.fen {
            Some(fen) => {
                VariantPosition::from_setup(variant, &Fen::from(fen), CastlingMode::Chess960)
                    .or_else(PositionError::ignore_invalid_castling_rights)
                    .or_else(PositionError::ignore_invalid_ep_square)?
            }
            None => VariantPosition::new(variant),
        });
        let opening = openings.classify_and_play(&mut pos, self.play)?;
        Ok((variant, pos, opening))
    }
}

#[serde_as]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "usize::max_value")]
    pub top_games: usize,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "usize::max_value")]
    pub recent_games: usize,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub moves: Option<usize>,
}
