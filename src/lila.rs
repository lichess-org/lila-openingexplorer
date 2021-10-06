use crate::model::{GameId, Speed};
use futures_util::stream::TryStreamExt as _;
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr, SpaceSeparator, StringWithSeparator};
use shakmaty::fen::Fen;
use shakmaty::san::San;
use std::io;
use tokio::io::AsyncBufReadExt as _;
use tokio_stream::wrappers::LinesStream;
use tokio_util::io::StreamReader;

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Game {
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
    #[serde(default)]
    winner: Option<WinnerColor>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    initial_fen: Option<Fen>,
}

#[derive(Deserialize)]
struct Players {
    white: Player,
    black: Player,
}

#[derive(Deserialize)]
struct Player {
    user: User,
    rating: u16,
}

#[derive(Deserialize)]
struct User {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum LilaVariant {
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

#[derive(Deserialize)]
enum WinnerColor {
    #[serde(rename = "white")]
    White,
    #[serde(rename = "black")]
    Black,
}

#[derive(Deserialize)]
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

pub struct Api {
    client: reqwest::Client,
}

impl Api {
    pub fn new() -> Api {
        Api {
            client: reqwest::Client::builder().build().expect("reqwest client"),
        }
    }

    pub async fn user_games(&self, name: String /* XXX */) -> reqwest::Result<()> {
        let stream = self
            .client
            .get(format!("https://lichess.org/api/games/user/{}", name))
            .header("Accept", "application/x-ndjson")
            .send()
            .await?
            .error_for_status()?
            .bytes_stream()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err));

        LinesStream::new(StreamReader::new(stream).lines());

        Ok(())
    }
}
