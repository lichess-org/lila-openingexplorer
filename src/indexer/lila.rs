use crate::{
    api::{Error, LilaVariant},
    indexer::IndexerOpt,
    model::{GameId, Speed, UserName},
};
use chrono::{DateTime, Utc};
use futures_util::stream::{Stream, StreamExt as _, TryStreamExt as _};
use serde::Deserialize;
use serde_with::{
    serde_as, DisplayFromStr, SpaceSeparator, StringWithSeparator, TimestampMilliSeconds,
};
use shakmaty::{fen::Fen, san::San, Color};
use std::io;
use tokio::io::AsyncBufReadExt as _;
use tokio_stream::wrappers::LinesStream;
use tokio_util::io::StreamReader;

pub struct Lila {
    client: reqwest::Client,
    opt: IndexerOpt,
}

impl Lila {
    pub fn new(opt: IndexerOpt) -> Lila {
        Lila {
            client: reqwest::Client::builder().build().expect("reqwest client"),
            opt,
        }
    }

    pub async fn user_games(
        &self,
        user: &UserName,
        since_created_at: u64,
    ) -> Result<impl Stream<Item = Result<Game, Error>>, Error> {
        // https://lichess.org/api#operation/apiGamesUser
        let stream = self
            .client
            .get(format!(
                "{}/api/games/user/{}?sort=dateAsc&ongoing=true",
                self.opt.lila, user
            ))
            .query(&[("since", since_created_at.saturating_sub(1))])
            .header("Accept", "application/x-ndjson")
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(Error::IndexerRequestError)?
            .bytes_stream()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err));

        Ok(Box::pin(
            LinesStream::new(StreamReader::new(stream).lines()).filter_map(|line| async move {
                match line {
                    Ok(line) if line.is_empty() => None,
                    Ok(line) => Some(serde_json::from_str::<Game>(&line).map_err(|err| {
                        Error::IndexerGameError {
                            err: err.into(),
                            line,
                        }
                    })),
                    Err(err) => Some(Err(Error::IndexerStreamError(err))),
                }
            }),
        ))
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    #[serde_as(as = "DisplayFromStr")]
    pub id: GameId,
    pub rated: bool,
    pub created_at: u64,
    #[serde_as(as = "TimestampMilliSeconds")]
    pub last_move_at: DateTime<Utc>,
    pub status: Status,
    pub variant: LilaVariant,
    pub players: Players,
    pub speed: Speed,
    #[serde_as(as = "StringWithSeparator::<SpaceSeparator, San>")]
    pub moves: Vec<San>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub winner: Option<Color>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub initial_fen: Option<Fen>,
}

impl Game {
    pub fn user_name(&self, color: Color) -> Option<&UserName> {
        self.players.by_color(color).user.as_ref().map(|u| &u.name)
    }
}

#[derive(Debug, Deserialize)]
pub struct Players {
    pub white: Player,
    pub black: Player,
}

impl Players {
    fn by_color(&self, color: Color) -> &Player {
        match color {
            Color::White => &self.white,
            Color::Black => &self.black,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Player {
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub rating: Option<u16>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct User {
    #[serde_as(as = "DisplayFromStr")]
    pub name: UserName,
}

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Created,
    Started,
    Aborted,
    Mate,
    Resign,
    Stalemate,
    Timeout,
    Draw,
    #[serde(rename = "outoftime")]
    OutOfTime,
    Cheat,
    NoStart,
    UnknownFinish,
    VariantEnd,
}

impl Status {
    pub fn is_ongoing(self) -> bool {
        matches!(self, Status::Created | Status::Started)
    }

    pub fn is_unindexable(self) -> bool {
        matches!(
            self,
            Status::UnknownFinish | Status::NoStart | Status::Aborted
        )
    }
}
