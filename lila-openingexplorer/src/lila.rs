use std::{io, time::SystemTime};

use clap::Parser;
use futures_util::stream::{Stream, StreamExt as _, TryStreamExt as _};
use serde::Deserialize;
use serde_with::{
    formats::SpaceSeparator, serde_as, DisplayFromStr, StringWithSeparator, TimestampMilliSeconds,
};
use shakmaty::{fen::Fen, san::San, variant::Variant, ByColor, Color};
use time::PrimitiveDateTime;
use tokio::io::AsyncBufReadExt as _;
use tokio_stream::wrappers::LinesStream;
use tokio_util::io::StreamReader;

use crate::{
    model::{GameId, Speed, UserId, UserName},
    util::ByColorDef,
};

#[derive(Parser, Clone)]
pub struct LilaOpt {
    /// Base url for the lila instance.
    #[arg(long = "lila", default_value = "https://lichess.org")]
    lila: String,
    /// Token of https://lichess.org/@/OpeningExplorer to speed up indexing
    /// and allow access to internal endpoints.
    #[arg(long = "bearer", env = "EXPLORER_BEARER")]
    bearer: Option<String>,
}

pub struct Lila {
    client: reqwest::Client,
    opt: LilaOpt,
}

impl Lila {
    pub fn new(opt: LilaOpt) -> Lila {
        Lila {
            client: reqwest::Client::builder()
                .user_agent("lila-openingexplorer")
                .build()
                .expect("reqwest client"),
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

    pub async fn mod_marked_since(
        &self,
        since: SystemTime,
    ) -> Result<impl Stream<Item = Result<UserId, io::Error>>, reqwest::Error> {
        let mut builder = self
            .client
            .get(format!("{}/api/stream/mod-marked-since", self.opt.lila))
            .query(&[(
                "since",
                since
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis()),
            )]);

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
                        line.parse::<UserName>()
                            .map(UserId::from)
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
    pub last_move_at: PrimitiveDateTime,
    pub status: Status,
    #[serde_as(as = "DisplayFromStr")]
    pub variant: Variant,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Month;

    #[test]
    fn test_deserialize_game() {
        let record = r#"{"id":"oV2tflO2","rated":false,"variant":"standard","speed":"rapid","perf":"rapid","createdAt":1651131730149,"lastMoveAt":1651131935030,"status":"mate","players":{"white":{"user":{"name":"revoof","patron":true,"id":"revoof"},"rating":1887,"provisional":true},"black":{"user":{"name":"maia1","title":"BOT","id":"maia1"},"rating":1417}},"winner":"white","moves":"d4 d5 c4 dxc4 Nc3 e6 e4 Nc6 Nf3 Bb4 Bxc4 Bxc3+ bxc3 Nf6 Bd3 O-O O-O b6 Bg5 Bb7 e5 h6 Bh4 g5 Nxg5 hxg5 Bxg5 Qd5 c4 Qxd4 Bxf6 Nxe5 Qh5 Ng6 Bxg6 fxg6 Qxg6#","clock":{"initial":600,"increment":0,"totalTime":600}}"#;

        let game: Game = serde_json::from_str(record).expect("deserialize");
        let month = Month::from_time_saturating(game.last_move_at);
        assert_eq!(month, Month::try_from(24267).unwrap());
    }
}
