//! Query parameters of `/player`.

use serde::Deserialize;
use serde_with::{DisplayFromStr, StringWithSeparator, formats::CommaSeparator, serde_as};
use shakmaty::Color;

use crate::{
    packed::{LaxMonth, Month, RatingMode, RatingModes, Speed, Speeds},
    player::UserId,
    server::query::Play,
};

/// Most recent games of a response.
pub const MAX_RECENT_GAMES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PlayerFilter {
    pub speeds: Option<Speeds>,
    pub modes: Option<RatingModes>,
    pub since: Option<Month>,
    pub until: Option<Month>,
}

impl PlayerFilter {
    pub fn contains_speed(&self, speed: Speed) -> bool {
        self.speeds
            .is_none_or(|speeds| speeds.contains(speed.into()))
    }

    pub fn contains_mode(&self, mode: RatingMode) -> bool {
        self.modes.is_none_or(|modes| modes.contains(mode.into()))
    }

    pub fn contains_month(&self, month: Month) -> bool {
        self.since.is_none_or(|since| since <= month)
            && self.until.is_none_or(|until| month <= until)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerLimits {
    pub moves: usize,
    pub recent_games: usize,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(try_from = "RawPlayerQuery")]
pub struct PlayerQuery {
    pub player: UserId,
    pub color: Color,
    pub play: Play,
    pub filter: PlayerFilter,
    pub limits: PlayerLimits,
}

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPlayerQuery {
    player: String,
    #[serde_as(as = "DisplayFromStr")]
    color: Color,
    #[serde(flatten)]
    play: Play,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, Speed>>")]
    speeds: Option<Speeds>,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, RatingMode>>")]
    modes: Option<RatingModes>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    since: Option<LaxMonth>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    until: Option<LaxMonth>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    moves: Option<usize>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    recent_games: Option<usize>,
}

impl TryFrom<RawPlayerQuery> for PlayerQuery {
    type Error = crate::player::InvalidUserName;

    fn try_from(raw: RawPlayerQuery) -> Result<PlayerQuery, Self::Error> {
        Ok(PlayerQuery {
            player: UserId::from_name(&raw.player)?,
            color: raw.color,
            play: raw.play,
            filter: PlayerFilter {
                speeds: raw.speeds,
                modes: raw.modes,
                since: raw.since.map(LaxMonth::earliest_month),
                until: raw.until.map(LaxMonth::latest_month),
            },
            limits: PlayerLimits {
                moves: raw.moves.unwrap_or(usize::MAX),
                recent_games: raw
                    .recent_games
                    .unwrap_or(MAX_RECENT_GAMES)
                    .min(MAX_RECENT_GAMES),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use axum::extract::Query;
    use shakmaty::variant::Variant;

    use super::*;

    fn player(query: &str) -> Result<PlayerQuery, String> {
        Query::try_from_uri(&format!("/player?{query}").parse().unwrap())
            .map(|Query(query)| query)
            .map_err(|err| err.body_text())
    }

    #[test]
    fn test_player_query() {
        let q = player(
            "player=Revoof&color=black&variant=atomic&modes=casual&speeds=blitz,rapid&since=2020-03&until=2021&recentGames=20&topGames=0&source=analysis",
        )
        .unwrap();
        assert_eq!(q.player.as_str(), "revoof");
        assert_eq!(q.color, Color::Black);
        assert_eq!(q.play.variant, Variant::Atomic);
        assert_eq!(q.filter.modes, Some(RatingModes::CASUAL));
        assert_eq!(q.filter.speeds, Some(Speeds::BLITZ | Speeds::RAPID));
        assert_eq!(q.filter.since.unwrap().to_string(), "2020-03");
        assert_eq!(q.filter.until.unwrap().to_string(), "2021-12");
        assert_eq!(q.limits.recent_games, MAX_RECENT_GAMES);
        assert_eq!(q.limits.moves, usize::MAX);

        let q = player("player=a&color=white&since=1952-01&until=3000-12").unwrap();
        assert!(
            q.filter
                .contains_month(Month::from_year_and_month(2026, 11).unwrap())
        );
        assert!(q.filter.contains_mode(RatingMode::Casual));

        assert!(player("player=a").is_err());
        assert!(player("color=white").is_err());
        assert!(player("player=a%20b&color=white").is_err());
        assert!(player("player=a&color=white&modes=unrated").is_err());
        assert!(player("player=a&color=white&speeds=blitz,hyperBullet").is_err());
    }
}
