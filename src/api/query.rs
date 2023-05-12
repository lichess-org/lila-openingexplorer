use std::{
    cmp::min,
    collections::BTreeSet,
    hash::{Hash, Hasher},
};

use serde::Deserialize;
use serde_with::{
    formats::CommaSeparator, serde_as, DisplayFromStr, StringWithSeparator, TryFromInto,
};
use shakmaty::{
    fen::Fen,
    uci::Uci,
    variant::{Variant, VariantPosition},
    CastlingMode, Color, EnPassantMode, Position, PositionError, Setup,
};

use crate::{
    api::Error,
    model::{Mode, Month, RatingGroup, Speed, UserName, Year},
    opening::{Opening, Openings},
};

#[serde_as]
#[derive(Deserialize, Clone, Debug, Eq, PartialEq, Hash)]
pub struct MastersQuery {
    #[serde(flatten)]
    pub play: Play,
    #[serde_as(as = "TryFromInto<u16>")]
    #[serde(default = "Year::min_value")]
    pub since: Year,
    #[serde_as(as = "TryFromInto<u16>")]
    #[serde(default = "Year::max_value")]
    pub until: Year,
    #[serde(flatten)]
    pub limits: Limits,
}

#[derive(Deserialize, Clone, Debug, Hash, Eq, PartialEq)]
pub struct LichessQuery {
    #[serde(flatten)]
    pub play: Play,
    #[serde(flatten)]
    pub limits: Limits,
    #[serde(flatten)]
    pub filter: LichessQueryFilter,
}

#[derive(Deserialize, Clone, Debug, Hash, Eq, PartialEq)]
pub struct LichessHistoryQuery {
    #[serde(flatten)]
    pub play: Play,
    #[serde(flatten)]
    pub filter: LichessQueryFilter,
}

#[serde_as]
#[derive(Deserialize, Clone, Debug, Hash, Eq, PartialEq)]
pub struct LichessQueryFilter {
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, Speed>>")]
    #[serde(default)]
    pub speeds: Option<BTreeSet<Speed>>,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, RatingGroup>>")]
    #[serde(default)]
    pub ratings: Option<BTreeSet<RatingGroup>>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub since: Option<Month>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub until: Option<Month>,
}

impl LichessQueryFilter {
    pub fn contains_speed(&self, speed: Speed) -> bool {
        self.speeds
            .as_ref()
            .map_or(true, |speeds| speeds.contains(&speed))
    }

    pub fn contains_rating_group(&self, rating_group: RatingGroup) -> bool {
        self.ratings.as_ref().map_or(true, |ratings| {
            ratings.contains(&min(rating_group, RatingGroup::Group2500))
        })
    }

    pub fn top_group(&self) -> Option<RatingGroup> {
        let mut top_group = None;
        for group in RatingGroup::ALL.into_iter().rev() {
            if !self.contains_rating_group(group) || group < RatingGroup::Group2000 {
                break;
            }
            top_group = Some(group);
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
    pub limits: PlayerLimits,
}

#[serde_as]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerLimits {
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "usize::max_value")]
    pub moves: usize,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "usize::max_value")]
    pub recent_games: usize,
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
    #[serde(default = "Month::min_value")]
    pub since: Month,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "Month::max_value")]
    pub until: Month,
}

#[serde_as]
#[derive(Deserialize, Clone, Debug, Eq)]
pub struct Play {
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default)]
    variant: Variant,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    fen: Option<Fen>,
    #[serde_as(as = "StringWithSeparator<CommaSeparator, Uci>")]
    #[serde(default)]
    play: Vec<Uci>,
}

impl Hash for Play {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.variant.hash(state);
        self.setup().hash(state);
        self.play.hash(state);
    }
}

impl PartialEq for Play {
    fn eq(&self, other: &Play) -> bool {
        self.variant == other.variant && self.setup() == other.setup() && self.play == other.play
    }
}

pub struct PlayPosition {
    pub pos: VariantPosition,
    pub opening: Option<Opening>,
}

impl Play {
    fn setup(&self) -> Setup {
        match self.fen {
            Some(ref fen) => fen.as_setup().to_owned(),
            None => VariantPosition::new(self.variant).into_setup(EnPassantMode::Always),
        }
    }

    pub fn position(self, openings: &Openings) -> Result<PlayPosition, Error> {
        let mut pos = match self.fen {
            Some(fen) => {
                VariantPosition::from_setup(self.variant, fen.into_setup(), CastlingMode::Chess960)
                    .or_else(PositionError::ignore_invalid_castling_rights)
                    .or_else(PositionError::ignore_invalid_ep_square)
                    .or_else(PositionError::ignore_too_much_material)?
            }
            None => VariantPosition::new(self.variant),
        };
        let opening = openings.classify_and_play(&mut pos, self.play)?;
        Ok(PlayPosition { pos, opening })
    }
}

#[serde_as]
#[derive(Deserialize, Clone, Debug, Eq, PartialEq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "usize::max_value")]
    pub top_games: usize,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "usize::max_value")]
    pub recent_games: usize,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(default = "Limits::default_moves")]
    pub moves: usize,
}

impl Limits {
    pub fn default_moves() -> usize {
        12
    }

    pub fn wants_games(&self) -> bool {
        self.top_games > 0 || self.recent_games > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_play_equality() {
        let a = Play {
            variant: Variant::Chess,
            fen: None,
            play: Vec::new(),
        };
        let b = Play {
            variant: Variant::Chess,
            fen: Some(Fen::default()),
            play: Vec::new(),
        };
        assert_eq!(a, b);
    }
}
