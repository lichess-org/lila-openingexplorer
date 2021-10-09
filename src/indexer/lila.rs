use crate::api::{ColorProxy, Error, LilaVariant};
use crate::model::{GameId, Speed, UserName};
use futures_util::stream::{Stream, StreamExt as _, TryStreamExt as _};
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr, FromInto, SpaceSeparator, StringWithSeparator};
use shakmaty::fen::Fen;
use shakmaty::san::San;
use shakmaty::Color;
use std::io;
use tokio::io::AsyncBufReadExt as _;
use tokio_stream::wrappers::LinesStream;
use tokio_util::io::StreamReader;

pub struct Lila {
    client: reqwest::Client,
}

impl Lila {
    pub fn new() -> Lila {
        Lila {
            client: reqwest::Client::builder().build().expect("reqwest client"),
        }
    }

    pub async fn user_games(
        &self,
        user: UserName,
    ) -> Result<impl Stream<Item = Result<Game, Error>>, Error> {
        let stream = self
            .client
            .get(format!("https://lichess.org/api/games/user/{}", user))
            .header("Accept", "application/x-ndjson")
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|err| Error::IndexerRequestError(err))?
            .bytes_stream()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err));

        Ok(Box::pin(
            LinesStream::new(StreamReader::new(stream).lines()).filter_map(|line| async move {
                match line {
                    Ok(line) if line.is_empty() => None,
                    Ok(line) => Some(
                        serde_json::from_str::<Game>(&line)
                            .map_err(|err| Error::IndexerStreamError(err.into())),
                    ),
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
    id: GameId,
    rated: bool,
    created_at: u64,
    status: Status,
    variant: LilaVariant,
    players: Players,
    speed: Speed,
    #[serde_as(as = "StringWithSeparator::<SpaceSeparator, San>")]
    moves: Vec<San>,
    #[serde_as(as = "Option<FromInto<ColorProxy>>")]
    #[serde(default)]
    winner: Option<Color>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    initial_fen: Option<Fen>,
}

#[derive(Debug, Deserialize)]
struct Players {
    white: Player,
    black: Player,
}

#[derive(Debug, Deserialize)]
struct Player {
    #[serde(default)]
    user: Option<User>,
    #[serde(default)]
    rating: Option<u16>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
struct User {
    #[serde_as(as = "DisplayFromStr")]
    name: UserName,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Status {
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
