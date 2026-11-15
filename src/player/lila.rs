use std::{sync::Arc, time::Duration};

use bytes::{Bytes, BytesMut};
use serde::Deserialize;
use shakmaty::{
    ByColor, CastlingMode, Color, KnownOutcome, Position, PositionError, fen::Fen, san::San,
    variant::VariantPosition,
};
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    lila_variant::LilaVariant,
    packed::{GameId, PackedMoves, Rating, RatingMode, Speed},
    player::{MAX_PLY, UserId, game::PlayerGame, time::UnixMillis},
};

#[derive(Debug, Clone)]
pub struct LilaClientConfig {
    /// Base url, like `https://lichess.org`.
    pub url: String,
    pub bearer: Option<String>,
    /// Parallel streams at most. We promise to lila that we will never open
    /// more concurrent streams.
    pub streams: usize,
    pub connect_timeout: Duration,
    /// Longest wait for the response headers (from sending the request)
    /// and between chunks of the body (reqwest's read timeout).
    pub read_timeout: Duration,
}

impl Default for LilaClientConfig {
    fn default() -> LilaClientConfig {
        LilaClientConfig {
            url: "https://lichess.org".to_owned(),
            bearer: None,
            streams: 8,
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Error)]
pub enum LilaError {
    #[error("not found")]
    NotFound,
    #[error("request: {0}")]
    Request(reqwest::Error),
    #[error("status {0}")]
    Status(reqwest::StatusCode),
    #[error("timeout")]
    Timeout,
}

impl From<reqwest::Error> for LilaError {
    fn from(err: reqwest::Error) -> LilaError {
        if err.is_timeout() {
            LilaError::Timeout
        } else {
            LilaError::Request(err)
        }
    }
}

pub struct LilaClient {
    client: reqwest::Client,
    config: LilaClientConfig,
    semaphore: Arc<Semaphore>,
}

impl LilaClient {
    pub fn new(config: LilaClientConfig) -> LilaClient {
        LilaClient {
            client: reqwest::Client::builder()
                .user_agent("lila-openingexplorer/4")
                .connect_timeout(config.connect_timeout)
                .read_timeout(config.read_timeout)
                .build()
                .expect("reqwest client"),
            semaphore: Arc::new(Semaphore::new(config.streams)),
            config,
        }
    }

    /// Waits for a free slot.
    pub async fn slot(self: &Arc<Self>) -> Slot {
        Slot {
            _permit: Arc::clone(&self.semaphore)
                .acquire_owned()
                .await
                .expect("semaphore not closed"),
            client: Arc::clone(self),
        }
    }

    /// Slots in use.
    pub fn busy(&self) -> usize {
        self.config.streams - self.semaphore.available_permits()
    }
}

/// Permission to have one stream from lila open. Requests borrow the slot
/// for the lifetime of their stream, so one slot never has two.
pub struct Slot {
    _permit: OwnedSemaphorePermit,
    client: Arc<LilaClient>,
}

impl Slot {
    /// Requests the games of a user created since a time, oldest
    /// first, including ongoing games.
    pub async fn user_games(
        &mut self,
        user: &UserId,
        since: UnixMillis,
    ) -> Result<GameStream<'_>, LilaError> {
        let config = &self.client.config;
        let mut request = self
            .client
            .client
            .get(format!(
                "{}/api/games/user/{}?sort=dateAsc&ongoing=true&since={since}",
                config.url,
                user.as_str()
            ))
            .header("Accept", "application/x-ndjson");
        if let Some(bearer) = &config.bearer {
            request = request.bearer_auth(bearer);
        }
        let response = request.send().await?;
        match response.status() {
            reqwest::StatusCode::NOT_FOUND => return Err(LilaError::NotFound),
            status if !status.is_success() => return Err(LilaError::Status(status)),
            _ => (),
        }
        Ok(GameStream {
            response,
            lines: Lines::default(),
            done: false,
            _slot: self,
        })
    }
}

/// The lines of a games export.
pub struct GameStream<'a> {
    response: reqwest::Response,
    lines: Lines,
    done: bool,
    _slot: &'a mut Slot,
}

impl GameStream<'_> {
    /// The next line that is not blank (without the newline), or `None` at
    /// the end.
    pub async fn next_line(&mut self) -> Result<Option<Bytes>, LilaError> {
        loop {
            if let Some(line) = self.lines.next_line() {
                if !line.trim_ascii().is_empty() {
                    return Ok(Some(line));
                }
                continue;
            }
            if self.done {
                return Ok(self
                    .lines
                    .finish()
                    .filter(|line| !line.trim_ascii().is_empty()));
            }
            match self.response.chunk().await? {
                Some(chunk) => self.lines.push(chunk),
                None => self.done = true,
            }
        }
    }
}

/// Splits chunks of a stream into lines (without the newline), scanning
/// each byte once, and copying only lines that span chunks.
#[derive(Default)]
struct Lines {
    /// The start of a line that spans chunks.
    partial: BytesMut,
    /// The rest of the current chunk.
    chunk: Bytes,
}

impl Lines {
    /// Adds the next chunk, after `next_line` returned `None`.
    fn push(&mut self, chunk: Bytes) {
        debug_assert!(self.chunk.is_empty());
        self.chunk = chunk;
    }

    /// The next complete line, or `None` if more chunks are needed.
    fn next_line(&mut self) -> Option<Bytes> {
        let Some(i) = memchr::memchr(b'\n', &self.chunk) else {
            self.partial.extend_from_slice(&self.chunk);
            self.chunk.clear();
            return None;
        };
        let mut line = self.chunk.split_to(i + 1);
        line.truncate(i);
        if self.partial.is_empty() {
            Some(line)
        } else {
            self.partial.extend_from_slice(&line);
            Some(self.partial.split().freeze())
        }
    }

    /// The last line, if the stream does not end with a newline.
    fn finish(&mut self) -> Option<Bytes> {
        debug_assert!(self.chunk.is_empty());
        (!self.partial.is_empty()).then(|| self.partial.split().freeze())
    }
}

#[derive(Debug, Deserialize)]
pub struct ExportUser {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExportPlayer {
    #[serde(default)]
    pub user: Option<ExportUser>,
    #[serde(default)]
    pub rating: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct ExportPlayers {
    pub white: ExportPlayer,
    pub black: ExportPlayer,
}

/// A game of lila's export, as far as it is needed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportGame {
    pub id: String,
    pub rated: bool,
    pub variant: String,
    pub speed: String,
    pub created_at: UnixMillis,
    pub last_move_at: UnixMillis,
    pub status: String,
    pub players: ExportPlayers,
    #[serde(default)]
    pub winner: Option<String>,
    #[serde(default)]
    pub moves: String,
    #[serde(default)]
    pub initial_fen: Option<String>,
}

/// Why a game of the export is not indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Not finished yet: to be revisited.
    Ongoing,
    /// Aborted, never started, or ended without result.
    Unfinished,
    /// A side has no account or no rating.
    Anonymous,
    /// The player is neither side.
    NotPlayer,
    /// Logged, with the reason.
    Invalid,
}

/// A game of the export, classified.
pub enum Classified {
    Game {
        game: Box<PlayerGame>,
        /// The name of the player as in the game.
        name: String,
    },
    Skip(Skip),
}

impl ExportGame {
    pub fn parse(line: &[u8]) -> Result<ExportGame, serde_json::Error> {
        serde_json::from_slice(line)
    }

    /// Classifies the game, replaying its first `MAX_PLY` plies for a
    /// `PlayerGame` of the user.
    pub fn classify(&self, user: &UserId) -> Classified {
        match self.status.as_str() {
            "created" | "started" => return Classified::Skip(Skip::Ongoing),
            "aborted" | "noStart" | "unknownFinish" => {
                return Classified::Skip(Skip::Unfinished);
            }
            _ => (),
        }
        let players = ByColor {
            white: &self.players.white,
            black: &self.players.black,
        };
        if players
            .iter()
            .any(|p| p.user.is_none() || p.rating.is_none())
        {
            return Classified::Skip(Skip::Anonymous);
        }
        let Some(color) = Color::ALL.into_iter().find(|&color| {
            players
                .get(color)
                .user
                .as_ref()
                .is_some_and(|u| user.is_name(&u.name))
        }) else {
            return Classified::Skip(Skip::NotPlayer);
        };
        let invalid = |reason: &str| {
            log::warn!("skipping game {} of {user}: {reason}", self.id);
            Classified::Skip(Skip::Invalid)
        };
        let Ok(id) = GameId::from_ascii(self.id.as_bytes()) else {
            return invalid("invalid id");
        };
        let Ok(variant) = LilaVariant::from_key(&self.variant).map(LilaVariant::variant) else {
            return invalid("unknown variant");
        };
        let Ok(speed) = Speed::from_ascii(self.speed.as_bytes()) else {
            return invalid("unknown speed");
        };
        let rating = |color: Color| {
            players
                .get(color)
                .rating
                .and_then(|r| Rating::try_from(i32::from(r)).ok())
        };
        let (Some(player_rating), Some(opponent_rating)) = (rating(color), rating(!color)) else {
            return invalid("invalid rating");
        };
        let mut pos = match &self.initial_fen {
            Some(fen) => {
                let Some(pos) = fen.parse::<Fen>().ok().and_then(|fen| {
                    VariantPosition::from_setup(variant, fen.into_setup(), CastlingMode::Chess960)
                        .or_else(PositionError::ignore_invalid_castling_rights)
                        .or_else(PositionError::ignore_invalid_ep_square)
                        .ok()
                }) else {
                    return invalid("invalid initial fen");
                };
                pos
            }
            None => VariantPosition::new(variant),
        };
        let setup = PlayerGame::setup_of(&pos);
        let mut moves = PackedMoves::encoder_with_capacity(MAX_PLY);
        for san in self.moves.split_ascii_whitespace().take(MAX_PLY) {
            // Cut off at the first illegal move, as before.
            let Ok(m) = san
                .parse::<San>()
                .map_err(|_| ())
                .and_then(|san| san.to_move(&pos).map_err(|_| ()))
            else {
                log::warn!("game {} of {user}: cut off at illegal move {san}", self.id);
                break;
            };
            moves.encode_next(&pos, m);
            pos.play_unchecked(m);
        }
        let opponent = players.get(!color).user.as_ref().expect("opponent account");
        let me = players.get(color).user.as_ref().expect("own account");
        Classified::Game {
            game: Box::new(PlayerGame {
                id,
                speed,
                mode: if self.rated {
                    RatingMode::Rated
                } else {
                    RatingMode::Casual
                },
                month: self.last_move_at.month(),
                outcome: match self.winner.as_deref() {
                    Some("white") => KnownOutcome::Decisive {
                        winner: Color::White,
                    },
                    Some("black") => KnownOutcome::Decisive {
                        winner: Color::Black,
                    },
                    _ => KnownOutcome::Draw,
                },
                color,
                player_rating,
                opponent_rating,
                opponent_name: opponent.name.clone(),
                opponent_title: opponent.title.clone().filter(|t| !t.is_empty()),
                player_title: me.title.clone().filter(|t| !t.is_empty()),
                setup,
                moves: moves.finish(),
            }),
            name: me.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    /// Lines are the same, whatever the chunk boundaries.
    #[test]
    fn test_lines() {
        arbtest(|u| {
            let data: Vec<u8> = (0..u.int_in_range(0..=200)?)
                .map(|_| u.choose(b"ab\n").copied())
                .collect::<arbitrary::Result<_>>()?;
            let mut expected: Vec<&[u8]> = data.split(|&c| c == b'\n').collect();
            if expected.last().is_some_and(|line| line.is_empty()) {
                expected.pop();
            }

            let mut lines = Lines::default();
            let mut actual = Vec::new();
            let mut rest = &data[..];
            while !rest.is_empty() {
                let (chunk, tail) = rest.split_at(u.int_in_range(1..=rest.len())?);
                rest = tail;
                lines.push(Bytes::copy_from_slice(chunk));
                while let Some(line) = lines.next_line() {
                    actual.push(line);
                }
            }
            actual.extend(lines.finish());
            assert_eq!(actual, expected);
            Ok(())
        });
    }

    #[test]
    fn test_classify() {
        let line = br#"{"id":"oV2tflO2","rated":false,"variant":"standard","speed":"rapid","perf":"rapid","createdAt":1651131730149,"lastMoveAt":1651131935030,"status":"mate","players":{"white":{"user":{"name":"revoof","patron":true,"id":"revoof"},"rating":1887,"provisional":true},"black":{"user":{"name":"maia1","title":"BOT","id":"maia1"},"rating":1417}},"winner":"white","moves":"d4 d5 c4 dxc4 Nc3 e6 e4 Nc6 Nf3 Bb4 Bxc4 Bxc3+ bxc3 Nf6 Bd3 O-O O-O b6 Bg5 Bb7 e5 h6 Bh4 g5 Nxg5 hxg5 Bxg5 Qd5 c4 Qxd4 Bxf6 Nxe5 Qh5 Ng6 Bxg6 fxg6 Qxg6#","clock":{"initial":600,"increment":0,"totalTime":600}}"#;
        let game = ExportGame::parse(line).unwrap();
        let Classified::Game { game, name } = game.classify(&UserId::from_name("Maia1").unwrap())
        else {
            panic!("expected game");
        };
        assert_eq!(name, "maia1");
        assert_eq!(game.color, Color::Black);
        assert_eq!(game.mode, RatingMode::Casual);
        assert_eq!(game.speed, Speed::Rapid);
        assert_eq!(game.month.to_string(), "2022-04");
        assert_eq!(game.opponent_name, "revoof");
        assert_eq!(game.player_title.as_deref(), Some("BOT"));
        assert_eq!(i32::from(game.opponent_rating), 1887);
        assert_eq!(setup_plies(&game), 37);

        let game = ExportGame::parse(line).unwrap();
        assert!(matches!(
            game.classify(&UserId::from_name("someone").unwrap()),
            Classified::Skip(Skip::NotPlayer)
        ));
        let mut game = ExportGame::parse(line).unwrap();
        game.status = "started".to_owned();
        assert!(matches!(
            game.classify(&UserId::from_name("revoof").unwrap()),
            Classified::Skip(Skip::Ongoing)
        ));
        // Cut off at an illegal move.
        let mut game = ExportGame::parse(line).unwrap();
        game.moves = "d4 d5 O-O-O c4".to_owned();
        let Classified::Game { game, .. } = game.classify(&UserId::from_name("revoof").unwrap())
        else {
            panic!("expected game");
        };
        assert_eq!(setup_plies(&game), 2);
    }

    fn setup_plies(game: &PlayerGame) -> usize {
        let pos = game.as_borrowed().initial_position();
        let mut decoder = game.moves.decode().unwrap();
        let mut pos = pos;
        let mut plies = 0;
        while let Some(m) = decoder.decode_next(&pos).unwrap() {
            pos.play_unchecked(m);
            plies += 1;
        }
        plies
    }
}
