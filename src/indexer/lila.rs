use std::io;

use chrono::{DateTime, Utc};
use futures_util::stream::{Stream, StreamExt as _, TryStreamExt as _};
use serde::Deserialize;
use serde_with::{
    serde_as, DisplayFromStr, SpaceSeparator, StringWithSeparator, TimestampMilliSeconds,
};
use shakmaty::{fen::Fen, san::San, ByColor, Color};
use tokio::io::AsyncBufReadExt as _;
use tokio_stream::wrappers::LinesStream;
use tokio_util::io::StreamReader;

use crate::{
    api::LilaVariant,
    indexer::IndexerOpt,
    model::{GameId, Speed, UserId, UserName},
    util::ByColorDef,
};

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
        user: &UserId,
        since_created_at: u64,
    ) -> Result<impl Stream<Item = Result<Game, io::Error>>, reqwest::Error> {
        // https://lichess.org/api#operation/apiGamesUser
        let mut builder = self
            .client
            .get(format!(
                "{}/api/games/user/{}?sort=dateAsc&ongoing=true",
                self.opt.lila,
                user.as_lowercase_str()
            ))
            .query(&[("since", since_created_at)])
            .header("Accept", "application/x-ndjson");

        if let Some(ref bearer) = self.opt.bearer {
            builder = builder.bearer_auth(bearer);
        }

        let stream = builder
            .send()
            .await
            .and_then(|r| r.error_for_status())?
            .bytes_stream()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err));

        Ok(Box::pin(
            LinesStream::new(StreamReader::new(stream).lines()).filter_map(|line| async move {
                match line {
                    Ok(line) if line.is_empty() => None,
                    Ok(line) => Some(
                        serde_json::from_str::<Game>(&line)
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err)),
                    ),
                    Err(err) => Some(Err(err)),
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
    #[serde(with = "ByColorDef")]
    pub players: ByColor<Player>,
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
