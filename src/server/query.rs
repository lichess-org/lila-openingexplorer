use axum::extract::{Query, rejection::QueryRejection};
use serde::Deserialize;
use serde_with::{
    DefaultOnError, DisplayFromStr, NoneAsEmptyString, StringWithSeparator,
    formats::CommaSeparator, serde_as,
};
use shakmaty::{
    CastlingMode, EnPassantMode, Position, PositionError, Setup,
    fen::Fen,
    uci::{IllegalUciMoveError, UciMove},
    variant::{Variant, VariantPosition},
};
use thiserror::Error;

use crate::{
    packed::{LaxMonth, Month, RatingGroup, RatingGroups, Speed, Speeds},
    server::openings::{Opening, Openings},
};

/// Bounds that clients send by default. Ranges reaching beyond them are
/// treated as unbounded, so that subtotals of the entries can be used.
const MIN_YEAR: u16 = 1952;
const MAX_YEAR: u16 = 2100;

fn since(since: Option<LaxMonth>) -> Option<Month> {
    since
        .map(LaxMonth::earliest_month)
        .filter(|&month| month > Month::from_year_and_month(MIN_YEAR, 1).expect("valid month"))
}

fn until(until: Option<LaxMonth>) -> Option<Month> {
    until
        .map(LaxMonth::latest_month)
        .filter(|&month| month < Month::from_year_and_month(MAX_YEAR, 12).expect("valid month"))
}

fn combine_top_groups(groups: RatingGroups) -> RatingGroups {
    // Top groups are always selected together.
    const TOP: RatingGroups = RatingGroups::GROUP_2500
        .union(RatingGroups::GROUP_2800)
        .union(RatingGroups::GROUP_3200);
    if groups.intersects(TOP) {
        groups | TOP
    } else {
        groups
    }
}

#[derive(Deserialize, Default, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum HistoryWanted {
    #[serde(alias = "false", alias = "False", alias = "off", alias = "0")]
    #[default]
    No,
    #[serde(alias = "true", alias = "True", alias = "on", alias = "1")]
    Yes,
}

/// Known clients, as given by the `source` query parameter.
#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Analysis,
    Fishnet,
    Opening,
    OpeningCrawler,
    Mobile,
}

impl Source {
    pub const ALL: [Source; 5] = [
        Source::Analysis,
        Source::Fishnet,
        Source::Opening,
        Source::OpeningCrawler,
        Source::Mobile,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Source::Analysis => "analysis",
            Source::Fishnet => "fishnet",
            Source::Opening => "opening",
            Source::OpeningCrawler => "openingCrawler",
            Source::Mobile => "mobile",
        }
    }
}

/// The `source` query parameter, separate from the query itself, so that it
/// does not split the response cache.
#[serde_as]
#[derive(Deserialize)]
pub struct SourceParam {
    #[serde_as(as = "DefaultOnError")]
    #[serde(default)]
    source: Option<Source>,
}

/// The source of a request, if it is a known one. Unknown, repeated or
/// absent sources are not rejected.
pub fn source(query: Result<Query<SourceParam>, QueryRejection>) -> Option<Source> {
    query.ok().and_then(|Query(param)| param.source)
}

#[derive(Debug, Error)]
pub enum PlayError {
    #[error("{0}")]
    Position(Box<PositionError<VariantPosition>>),
    #[error("illegal move {uci}: {source}")]
    IllegalMove {
        uci: UciMove,
        source: IllegalUciMoveError,
    },
}

impl From<PositionError<VariantPosition>> for PlayError {
    fn from(err: PositionError<VariantPosition>) -> PlayError {
        PlayError::Position(Box::new(err))
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(from = "RawPlay")]
pub struct Play {
    pub variant: Variant,
    pub setup: Setup,
    pub play: Vec<UciMove>,
}

#[serde_as]
#[derive(Deserialize)]
struct RawPlay {
    #[serde_as(as = "Option<DisplayFromStr>")]
    variant: Option<Variant>,
    #[serde_as(as = "NoneAsEmptyString")]
    #[serde(default)]
    fen: Option<Fen>,
    #[serde_as(as = "StringWithSeparator<CommaSeparator, UciMove>")]
    #[serde(default)]
    play: Vec<UciMove>,
}

impl From<RawPlay> for Play {
    fn from(raw: RawPlay) -> Play {
        let variant = raw.variant.unwrap_or(Variant::Chess);
        Play {
            variant,
            setup: match raw.fen {
                Some(fen) => fen.into_setup(),
                None => VariantPosition::new(variant).to_setup(EnPassantMode::Legal),
            },
            play: raw.play,
        }
    }
}

pub struct PlayPosition {
    pub pos: VariantPosition,
    pub opening: Option<Opening>,
}

impl Play {
    pub fn position(&self, openings: &Openings) -> Result<PlayPosition, PlayError> {
        let mut pos =
            VariantPosition::from_setup(self.variant, self.setup.clone(), CastlingMode::Chess960)
                .or_else(PositionError::ignore_invalid_castling_rights)
                .or_else(PositionError::ignore_invalid_ep_square)
                .or_else(PositionError::ignore_too_much_material)?;
        let mut opening = openings.classify_exact(&pos);
        for uci in &self.play {
            let m = uci
                .to_move(&pos)
                .map_err(|source| PlayError::IllegalMove { uci: *uci, source })?;
            pos.play_unchecked(m);
            opening = openings.classify_exact(&pos).or(opening);
        }
        Ok(PlayPosition {
            pos,
            opening: opening.cloned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LichessFilter {
    pub speeds: Option<Speeds>,
    pub ratings: Option<RatingGroups>,
    pub since: Option<Month>,
    pub until: Option<Month>,
}

impl LichessFilter {
    pub fn contains_speed(&self, speed: Option<Speed>) -> bool {
        match self.speeds {
            None => true,
            Some(speeds) => speed.is_some_and(|speed| speeds.contains(speed.into())),
        }
    }

    pub fn contains_rating_group(&self, group: RatingGroup) -> bool {
        self.ratings
            .is_none_or(|ratings| ratings.contains(group.into()))
    }

    pub fn contains_month(&self, month: Month) -> bool {
        self.since.is_none_or(|since| since <= month)
            && self.until.is_none_or(|until| month <= until)
    }

    /// The lowest of the selected top rating groups, if the selection
    /// includes the top groups contiguously down to at least 2000.
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Limits {
    pub moves: usize,
    pub top_games: usize,
    pub recent_games: usize,
}

impl Limits {
    pub const DEFAULT_MOVES: usize = 12;

    pub fn games_wanted(&self) -> bool {
        self.top_games > 0 || self.recent_games > 0
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(from = "RawLichessQuery")]
pub struct LichessQuery {
    pub play: Play,
    pub filter: LichessFilter,
    pub limits: Limits,
    pub history: bool,
}

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLichessQuery {
    #[serde(flatten)]
    play: Play,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, Speed>>")]
    speeds: Option<Speeds>,
    #[serde_as(as = "Option<StringWithSeparator<CommaSeparator, RatingGroup>>")]
    ratings: Option<RatingGroups>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    since: Option<LaxMonth>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    until: Option<LaxMonth>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    moves: Option<usize>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    top_games: Option<usize>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    recent_games: Option<usize>,
    #[serde(default)]
    history: HistoryWanted,
}

impl From<RawLichessQuery> for LichessQuery {
    fn from(raw: RawLichessQuery) -> LichessQuery {
        LichessQuery {
            play: raw.play,
            filter: LichessFilter {
                speeds: raw.speeds,
                ratings: raw.ratings.map(combine_top_groups),
                since: since(raw.since),
                until: until(raw.until),
            },
            limits: Limits {
                moves: raw.moves.unwrap_or(Limits::DEFAULT_MOVES),
                top_games: raw.top_games.unwrap_or(usize::MAX),
                recent_games: raw.recent_games.unwrap_or(usize::MAX),
            },
            history: raw.history == HistoryWanted::Yes,
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(from = "RawMastersQuery")]
pub struct MastersQuery {
    pub play: Play,
    pub since: Option<Month>,
    pub until: Option<Month>,
    pub limits: Limits,
}

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMastersQuery {
    #[serde(flatten)]
    play: Play,
    #[serde_as(as = "Option<DisplayFromStr>")]
    since: Option<LaxMonth>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    until: Option<LaxMonth>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    moves: Option<usize>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    top_games: Option<usize>,
}

impl From<RawMastersQuery> for MastersQuery {
    fn from(raw: RawMastersQuery) -> MastersQuery {
        MastersQuery {
            play: raw.play,
            since: since(raw.since),
            until: until(raw.until),
            limits: Limits {
                moves: raw.moves.unwrap_or(Limits::DEFAULT_MOVES),
                top_games: raw.top_games.unwrap_or(usize::MAX),
                recent_games: 0,
            },
        }
    }
}

impl MastersQuery {
    pub fn filter(&self) -> LichessFilter {
        LichessFilter {
            speeds: None,
            ratings: None,
            since: self.since,
            until: self.until,
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::extract::Query;

    use super::*;

    fn lichess(query: &str) -> Result<LichessQuery, String> {
        Query::try_from_uri(&format!("/lichess?{query}").parse().unwrap())
            .map(|Query(query)| query)
            .map_err(|err| err.body_text())
    }

    fn masters(query: &str) -> Result<MastersQuery, String> {
        Query::try_from_uri(&format!("/masters?{query}").parse().unwrap())
            .map(|Query(query)| query)
            .map_err(|err| err.body_text())
    }

    #[test]
    fn test_top_group() {
        let filter = |ratings: &str| lichess(&format!("ratings={ratings}")).unwrap().filter;
        assert_eq!(filter("2500").top_group(), Some(RatingGroup::Group2500));
        assert_eq!(
            filter("2000,2200,2500").top_group(),
            Some(RatingGroup::Group2000)
        );
        assert_eq!(
            filter("1800,2000,2200,2500").top_group(),
            Some(RatingGroup::Group2000)
        );
        assert_eq!(filter("2000,2200").top_group(), None);
        assert_eq!(filter("1600").top_group(), None);
        assert!(filter("2500").contains_rating_group(RatingGroup::Group3200));
        // Groups from 2500 are selected together.
        assert_eq!(filter("2900"), filter("2500"));
        assert!(filter("2800").contains_rating_group(RatingGroup::Group3200));
        assert!(!filter("2800").contains_rating_group(RatingGroup::Group2200));
        assert!(filter("0").contains_rating_group(RatingGroup::Low));
        assert_eq!(filter("400"), filter("0"));
        assert!(lichess("ratings=-1,1600").is_ok());
        assert!(lichess("ratings=x").is_err());
    }

    fn month(year: u16, month: u16) -> Option<Month> {
        Some(Month::from_year_and_month(year, month).unwrap())
    }

    #[test]
    fn test_months() {
        let range = |query: &str| {
            let filter = lichess(query)?.filter;
            Ok::<_, String>((filter.since, filter.until))
        };
        assert_eq!(
            range("since=2020-03&until=2021").unwrap(),
            (month(2020, 3), month(2021, 12))
        );
        assert_eq!(
            range("since=2020&until=2020/03").unwrap(),
            (month(2020, 1), month(2020, 3))
        );
        // Bounds beyond the defaults are unbounded.
        assert_eq!(range("since=1952-01&until=3000-12").unwrap(), (None, None));
        assert_eq!(range("since=1900&until=3001").unwrap(), (None, None));
        assert_eq!(range("since=1952-02").unwrap(), (month(1952, 2), None));
        assert!(range("since=1899-12").is_err());
        assert!(range("since=2020-13").is_err());
        assert!(range("since=2020-").is_err());
        assert!(range("until=9999").is_err());
    }

    #[test]
    fn test_variants() {
        for (variant, expected) in [
            ("standard", Variant::Chess),
            ("chess960", Variant::Chess),
            ("fromPosition", Variant::Chess),
            ("crazyhouse", Variant::Crazyhouse),
        ] {
            assert_eq!(
                lichess(&format!("variant={variant}")).unwrap().play.variant,
                expected
            );
        }
        // Chess960 positions with castling rights in X-FEN notation.
        let q = lichess(
            "variant=chess960&fen=bbnrnkrq%2Fpppppppp%2F8%2F8%2F8%2F8%2FPPPPPPPP%2FBBNRNKRQ+w+KQkq+-+0+1&play=e1f3",
        )
        .unwrap();
        assert!(q.play.position(&Openings::default()).is_ok());
    }

    #[test]
    fn test_play() {
        let q = lichess(
            "variant=standard&fen=rnbqkbnr%2Fpppppppp%2F8%2F8%2F8%2F8%2FPPPPPPPP%2FRNBQKBNR+w+KQkq+-+0+1&play=e2e4%2Ce7e5",
        )
        .unwrap();
        let pos = q.play.position(&Openings::default()).unwrap();
        assert_eq!(pos.pos.fullmoves().get(), 2);
        assert_eq!(q.play, lichess("play=e2e4,e7e5").unwrap().play);
        assert_eq!(
            lichess("fen=&play=").unwrap().play,
            lichess("").unwrap().play
        );
        assert!(
            lichess("play=e2e5")
                .unwrap()
                .play
                .position(&Openings::default())
                .is_err()
        );
        assert!(lichess("play=e2").is_err());
        assert!(lichess("variant=chess961").is_err());
    }

    #[test]
    fn test_lichess_query() {
        let q =
            lichess("speeds=blitz,rapid&history=yes&moves=3&topGames=0&source=analysis").unwrap();
        assert_eq!(q.filter.speeds, Some(Speeds::BLITZ | Speeds::RAPID));
        assert!(q.history);
        assert_eq!(
            q.limits,
            Limits {
                moves: 3,
                top_games: 0,
                recent_games: usize::MAX,
            }
        );
        assert!(lichess("history=1").unwrap().history);
        assert!(!lichess("history=False").unwrap().history);
        assert!(!lichess("history=no").unwrap().history);
        assert!(lichess("history=maybe").is_err());
        assert!(lichess("speeds=blitz,hyperBullet").is_err());
        assert!(lichess("moves=-1").is_err());
    }

    #[test]
    fn test_source() {
        let source_of = |query: &str| {
            source(Query::try_from_uri(
                &format!("/lichess?{query}").parse().unwrap(),
            ))
        };
        assert_eq!(source_of("source=fishnet"), Some(Source::Fishnet));
        assert_eq!(
            source_of("play=e2e4&source=openingCrawler"),
            Some(Source::OpeningCrawler)
        );
        assert_eq!(source_of(""), None);
        assert_eq!(source_of("source="), None);
        assert_eq!(source_of("source=foo"), None);
    }

    #[test]
    fn test_masters_query() {
        let q = masters("since=2000&until=2010&recentGames=4").unwrap();
        assert_eq!((q.since, q.until), (month(2000, 1), month(2010, 12)));
        assert_eq!(q.limits.recent_games, 0);
        let q = masters("since=1952&until=2010-06").unwrap();
        assert_eq!((q.since, q.until), (None, month(2010, 6)));
        assert!(masters("since=x").is_err());
    }
}
