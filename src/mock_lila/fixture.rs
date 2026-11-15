//! Fixture of the mock lila: the games of a set of players, grouped by
//! player and ordered by `createdAt`.
//!
//! Files of a fixture directory:
//!
//! ```text
//! games.bin    := record*                    (contiguous per player)
//! record       := u32(len) body              (big endian, len of body)
//! body         := u64(createdAt) GameId variant speed outcome status
//!                 u16(plies) player player string(initialFen) string(sans)
//! player       := string(name) string(title) u16(rating, 0 if none)
//! string       := uint(len) bytes
//! games.idx    := (u64(createdAt) u64(offset of record))*  (same order)
//! players.idx  := (u8(len) id u64(first game) u32(games))*  (sorted by id)
//! meta.json    := FixtureMeta
//! ```
//!
//! `createdAt` is in milliseconds since the epoch, before the time shift
//! that the mock applies when serving.

use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    os::unix::fs::FileExt,
    path::Path,
};

use bytes::{Buf, BufMut};
use serde::{Deserialize, Serialize};
use shakmaty::{ByColor, Color, KnownOutcome};
use thiserror::Error;

use crate::{
    lila_variant::LilaVariant,
    packed::{GameId, Speed, outcome_bits, outcome_from_bits, pack_uint, unpack_uint},
    player::UserId,
};

/// Variants in the fixture, by position in `LilaVariant::ALL`.
fn variant_bits(variant: LilaVariant) -> u8 {
    LilaVariant::ALL
        .iter()
        .position(|&v| v == variant)
        .expect("variant") as u8
}

fn variant_from_bits(bits: u8) -> Option<LilaVariant> {
    LilaVariant::ALL.get(usize::from(bits)).copied()
}

/// How a finished game ended (`status` in lila's exports).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum FinalStatus {
    Mate,
    Resign,
    Stalemate,
    OutOfTime,
    Draw,
    VariantEnd,
}

impl FinalStatus {
    const ALL: [FinalStatus; 6] = [
        FinalStatus::Mate,
        FinalStatus::Resign,
        FinalStatus::Stalemate,
        FinalStatus::OutOfTime,
        FinalStatus::Draw,
        FinalStatus::VariantEnd,
    ];

    pub fn key(self) -> &'static str {
        match self {
            FinalStatus::Mate => "mate",
            FinalStatus::Resign => "resign",
            FinalStatus::Stalemate => "stalemate",
            FinalStatus::OutOfTime => "outoftime",
            FinalStatus::Draw => "draw",
            FinalStatus::VariantEnd => "variantEnd",
        }
    }

    fn bits(self) -> u8 {
        FinalStatus::ALL
            .iter()
            .position(|&s| s == self)
            .expect("status") as u8
    }

    fn from_bits(bits: u8) -> Option<FinalStatus> {
        FinalStatus::ALL.get(usize::from(bits)).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixturePlayer {
    pub name: String,
    pub title: Option<String>,
    pub rating: Option<u16>,
}

/// A game of the fixture, as it was played to the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureGame {
    pub created_at: u64,
    pub id: GameId,
    pub variant: LilaVariant,
    pub speed: Speed,
    pub outcome: KnownOutcome,
    pub status: FinalStatus,
    pub players: ByColor<FixturePlayer>,
    pub initial_fen: Option<String>,
    /// SAN of all moves, space separated.
    pub sans: String,
}

impl FixtureGame {
    pub fn plies(&self) -> usize {
        self.sans.split_ascii_whitespace().count()
    }

    pub fn pack(&self, buf: &mut Vec<u8>) {
        let start = buf.len();
        buf.put_u32(0);
        buf.put_u64(self.created_at);
        self.id.pack(buf);
        buf.put_u8(variant_bits(self.variant));
        buf.put_u8(self.speed.bits());
        buf.put_u8(outcome_bits(self.outcome));
        buf.put_u8(self.status.bits());
        buf.put_u16(u16::try_from(self.plies()).unwrap_or(u16::MAX));
        for color in Color::ALL {
            let player = self.players.get(color);
            pack_str(buf, &player.name);
            pack_str(buf, player.title.as_deref().unwrap_or(""));
            buf.put_u16(player.rating.unwrap_or(0));
        }
        pack_str(buf, self.initial_fen.as_deref().unwrap_or(""));
        pack_str(buf, &self.sans);
        let len = u32::try_from(buf.len() - start - 4).expect("record size");
        buf[start..start + 4].copy_from_slice(&len.to_be_bytes());
    }

    /// Unpacks the body of a record (without the length prefix).
    pub fn unpack_body(mut buf: &[u8]) -> Result<FixtureGame, FixtureError> {
        let buf = &mut buf;
        if buf.remaining() < 8 + GameId::SIZE + 6 {
            return Err(FixtureError::Corrupt);
        }
        let created_at = buf.get_u64();
        let mut id = [0; GameId::SIZE];
        buf.copy_to_slice(&mut id);
        let id = GameId::try_from(id).map_err(|_| FixtureError::Corrupt)?;
        let variant = variant_from_bits(buf.get_u8()).ok_or(FixtureError::Corrupt)?;
        let speed = Speed::from_bits(buf.get_u8()).ok_or(FixtureError::Corrupt)?;
        let outcome = outcome_from_bits(buf.get_u8()).ok_or(FixtureError::Corrupt)?;
        let status = FinalStatus::from_bits(buf.get_u8()).ok_or(FixtureError::Corrupt)?;
        let _plies = buf.get_u16();
        let mut player = || -> Result<FixturePlayer, FixtureError> {
            let name = unpack_str(buf)?.to_owned();
            let title = Some(unpack_str(buf)?.to_owned()).filter(|t| !t.is_empty());
            if buf.remaining() < 2 {
                return Err(FixtureError::Corrupt);
            }
            let rating = Some(buf.get_u16()).filter(|&r| r != 0);
            Ok(FixturePlayer {
                name,
                title,
                rating,
            })
        };
        let white = player()?;
        let black = player()?;
        let initial_fen = Some(unpack_str(buf)?.to_owned()).filter(|f| !f.is_empty());
        let sans = unpack_str(buf)?.to_owned();
        Ok(FixtureGame {
            created_at,
            id,
            variant,
            speed,
            outcome,
            status,
            players: ByColor { white, black },
            initial_fen,
            sans,
        })
    }
}

fn pack_str(buf: &mut Vec<u8>, s: &str) {
    pack_uint(buf, s.len() as u64);
    buf.put_slice(s.as_bytes());
}

fn unpack_str<'a>(buf: &mut &'a [u8]) -> Result<&'a str, FixtureError> {
    let len = usize::try_from(unpack_uint(buf)).map_err(|_| FixtureError::Corrupt)?;
    if buf.len() < len {
        return Err(FixtureError::Corrupt);
    }
    let (s, rest) = buf.split_at(len);
    *buf = rest;
    std::str::from_utf8(s).map_err(|_| FixtureError::Corrupt)
}

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("corrupt fixture")]
    Corrupt,
    #[error("players must be written in order of their ids, without duplicates")]
    PlayerOrder,
    #[error("games of a player must be in order of createdAt")]
    GameOrder,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FixtureMeta {
    pub players: u64,
    pub games: u64,
    /// Latest `createdAt` of any game.
    pub max_created_at: u64,
}

/// Writes a fixture directory, player by player.
pub struct FixtureWriter {
    dir: std::path::PathBuf,
    games: BufWriter<File>,
    games_idx: BufWriter<File>,
    players_idx: BufWriter<File>,
    offset: u64,
    meta: FixtureMeta,
    last_player: Option<UserId>,
    buf: Vec<u8>,
}

impl FixtureWriter {
    pub fn create(dir: &Path) -> Result<FixtureWriter, FixtureError> {
        fs::create_dir_all(dir)?;
        let create = |name: &str| -> io::Result<BufWriter<File>> {
            Ok(BufWriter::with_capacity(
                1 << 20,
                File::create(dir.join(name))?,
            ))
        };
        Ok(FixtureWriter {
            dir: dir.to_owned(),
            games: create("games.bin")?,
            games_idx: create("games.idx")?,
            players_idx: create("players.idx")?,
            offset: 0,
            meta: FixtureMeta::default(),
            last_player: None,
            buf: Vec::new(),
        })
    }

    /// Writes all games of a player, in order of `createdAt` (each game
    /// already packed with `FixtureGame::pack`). Players must come in order
    /// of their ids.
    pub fn write_player<'a>(
        &mut self,
        player: &UserId,
        games: impl IntoIterator<Item = &'a [u8]>,
    ) -> Result<(), FixtureError> {
        if self.last_player.as_ref().is_some_and(|last| last >= player) {
            return Err(FixtureError::PlayerOrder);
        }
        self.last_player = Some(player.clone());
        let first = self.meta.games;
        let mut previous = None;
        for record in games {
            let created_at = (&record[4..]).get_u64();
            if previous.is_some_and(|previous| previous > created_at) {
                return Err(FixtureError::GameOrder);
            }
            previous = Some(created_at);
            self.meta.max_created_at = self.meta.max_created_at.max(created_at);
            self.games.write_all(record)?;
            self.games_idx.write_all(&created_at.to_be_bytes())?;
            self.games_idx.write_all(&self.offset.to_be_bytes())?;
            self.offset += record.len() as u64;
            self.meta.games += 1;
        }
        self.buf.clear();
        self.buf.put_u8(player.as_str().len() as u8);
        self.buf.put_slice(player.as_str().as_bytes());
        self.buf.put_u64(first);
        self.buf
            .put_u32(u32::try_from(self.meta.games - first).expect("games per player"));
        self.players_idx.write_all(&self.buf)?;
        self.meta.players += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<FixtureMeta, FixtureError> {
        self.games.flush()?;
        self.games_idx.flush()?;
        self.players_idx.flush()?;
        fs::write(
            self.dir.join("meta.json"),
            serde_json::to_vec_pretty(&self.meta)?,
        )?;
        Ok(self.meta)
    }
}

/// The games of a player in the fixture: a range of game numbers.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PlayerGames {
    pub first: u64,
    pub count: u32,
}

/// A fixture opened for serving: the indexes in memory, records read from
/// the file on demand.
pub struct Fixture {
    games: File,
    /// `createdAt` and offset of each game.
    index: Vec<(u64, u64)>,
    games_len: u64,
    players: Vec<(UserId, PlayerGames)>,
    pub meta: FixtureMeta,
}

impl Fixture {
    pub fn open(dir: &Path) -> Result<Fixture, FixtureError> {
        let meta: FixtureMeta = serde_json::from_slice(&fs::read(dir.join("meta.json"))?)?;
        let games = File::open(dir.join("games.bin"))?;
        let games_len = games.metadata()?.len();
        let raw = fs::read(dir.join("games.idx"))?;
        if raw.len() % 16 != 0 {
            return Err(FixtureError::Corrupt);
        }
        let index = raw
            .as_chunks::<16>()
            .0
            .iter()
            .map(|chunk| {
                let mut chunk = &chunk[..];
                (chunk.get_u64(), chunk.get_u64())
            })
            .collect();
        let raw = fs::read(dir.join("players.idx"))?;
        let mut buf = &raw[..];
        let mut players = Vec::new();
        while buf.has_remaining() {
            let len = usize::from(buf.get_u8());
            if buf.remaining() < len + 12 {
                return Err(FixtureError::Corrupt);
            }
            let id = std::str::from_utf8(&buf[..len]).map_err(|_| FixtureError::Corrupt)?;
            let id = UserId::from_name(id).map_err(|_| FixtureError::Corrupt)?;
            buf.advance(len);
            let first = buf.get_u64();
            let count = buf.get_u32();
            players.push((id, PlayerGames { first, count }));
        }
        if !players.is_sorted_by(|a, b| a.0 < b.0) {
            return Err(FixtureError::PlayerOrder);
        }
        Ok(Fixture {
            games,
            index,
            games_len,
            players,
            meta,
        })
    }

    pub fn players(&self) -> impl Iterator<Item = (&UserId, PlayerGames)> {
        self.players.iter().map(|(id, games)| (id, *games))
    }

    pub fn player(&self, id: &UserId) -> Option<PlayerGames> {
        self.players
            .binary_search_by(|(other, _)| other.cmp(id))
            .ok()
            .map(|i| self.players[i].1)
    }

    pub fn created_at(&self, game: u64) -> u64 {
        self.index[game as usize].0
    }

    /// The first game of the player with `createdAt >= since`.
    pub fn lower_bound(&self, games: PlayerGames, since: u64) -> u64 {
        let range =
            &self.index[games.first as usize..(games.first + u64::from(games.count)) as usize];
        games.first + range.partition_point(|&(created_at, _)| created_at < since) as u64
    }

    /// Reads the packed records of consecutive games `start..end` (with
    /// length prefixes) with a single read.
    pub fn read_records(&self, start: u64, end: u64) -> io::Result<Vec<u8>> {
        if start >= end {
            return Ok(Vec::new());
        }
        let from = self.index[start as usize].1;
        let to = self
            .index
            .get(end as usize)
            .map_or(self.games_len, |&(_, offset)| offset);
        let mut buf = vec![0; (to - from) as usize];
        self.games.read_exact_at(&mut buf, from)?;
        Ok(buf)
    }
}

/// Writes a fixture of random games (for tests): `games` games per player,
/// one per hour from `start` (ms), against opponents that are not in the
/// fixture. Returns the games of each player.
pub fn write_synthetic(
    dir: &Path,
    players: &[UserId],
    games: impl Fn(usize) -> usize,
    start: u64,
    seed: u64,
) -> Result<Vec<Vec<FixtureGame>>, FixtureError> {
    use shakmaty::{Chess, Position, san::SanPlus};

    let mut x = seed ^ 0x2545_f491_4f6c_dd1d;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut players: Vec<UserId> = players.to_vec();
    players.sort();
    let mut writer = FixtureWriter::create(dir)?;
    let mut all = Vec::new();
    let mut id = 0u64;
    for (p, player) in players.iter().enumerate() {
        let mut records = Vec::new();
        let mut list = Vec::new();
        for g in 0..games(p) {
            let mut pos = Chess::default();
            let mut sans = Vec::new();
            for ply in 0..(next() % 80) {
                let legals = pos.legal_moves();
                if legals.is_empty() {
                    break;
                }
                let i = if ply < 6 { next() % 3 } else { next() } as usize % legals.len();
                sans.push(SanPlus::from_move_and_play_unchecked(&mut pos, legals[i]).to_string());
            }
            let me = FixturePlayer {
                name: player.as_str().to_uppercase(),
                title: None,
                rating: Some(1500 + (next() % 500) as u16),
            };
            let opponent = FixturePlayer {
                name: format!("opponent{}", next() % 50),
                title: (next() % 10 == 0).then(|| "FM".to_owned()),
                rating: Some(1200 + (next() % 1000) as u16),
            };
            let white = next() % 2 == 0;
            id += 1;
            let mut digits = [b'0'; 8];
            let mut n = id + p as u64 * 1_000_000;
            for d in digits.iter_mut().rev() {
                *d = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
                    [(n % 62) as usize];
                n /= 62;
            }
            let game = FixtureGame {
                created_at: start + g as u64 * 3_600_000,
                id: GameId::from_ascii(&digits).expect("game id"),
                variant: LilaVariant::Standard,
                speed: Speed::from_bits(1 + (next() % 6) as u8).expect("speed"),
                outcome: match next() % 3 {
                    0 => KnownOutcome::Draw,
                    1 => KnownOutcome::Decisive {
                        winner: Color::White,
                    },
                    _ => KnownOutcome::Decisive {
                        winner: Color::Black,
                    },
                },
                status: FinalStatus::Resign,
                players: if white {
                    ByColor {
                        white: me,
                        black: opponent,
                    }
                } else {
                    ByColor {
                        white: opponent,
                        black: me,
                    }
                },
                initial_fen: None,
                sans: sans.join(" "),
            };
            let mut buf = Vec::new();
            game.pack(&mut buf);
            records.push(buf);
            list.push(game);
        }
        writer.write_player(player, records.iter().map(Vec::as_slice))?;
        all.push(list);
    }
    writer.finish()?;
    Ok(all)
}

/// Iterates the bodies of packed records.
pub fn record_bodies(mut buf: &[u8]) -> impl Iterator<Item = &[u8]> {
    std::iter::from_fn(move || {
        if buf.len() < 4 {
            return None;
        }
        let len = (&buf[..4]).get_u32() as usize;
        let body = buf.get(4..4 + len)?;
        buf = &buf[4 + len..];
        Some(body)
    })
}

#[cfg(test)]
mod tests {
    use shakmaty::Color;

    use super::*;

    fn game(created_at: u64, id: &str) -> FixtureGame {
        FixtureGame {
            created_at,
            id: id.parse().unwrap(),
            variant: LilaVariant::Standard,
            speed: Speed::Blitz,
            outcome: KnownOutcome::Decisive {
                winner: Color::White,
            },
            status: FinalStatus::Resign,
            players: ByColor {
                white: FixturePlayer {
                    name: "Alice".to_owned(),
                    title: Some("GM".to_owned()),
                    rating: Some(2500),
                },
                black: FixturePlayer {
                    name: "bob".to_owned(),
                    title: None,
                    rating: None,
                },
            },
            initial_fen: None,
            sans: "e4 e5 Qh5 Nc6".to_owned(),
        }
    }

    #[test]
    fn test_roundtrip() {
        let dir = std::env::temp_dir().join(format!("fixture-test-{}", std::process::id()));
        let mut writer = FixtureWriter::create(&dir).unwrap();
        let games = [
            game(10, "aaaaaaaa"),
            game(20, "bbbbbbbb"),
            game(30, "cccccccc"),
        ];
        let packed: Vec<Vec<u8>> = games
            .iter()
            .map(|g| {
                let mut buf = Vec::new();
                g.pack(&mut buf);
                buf
            })
            .collect();
        let alice = UserId::from_name("alice").unwrap();
        let bob = UserId::from_name("bob").unwrap();
        writer
            .write_player(&alice, packed.iter().map(Vec::as_slice))
            .unwrap();
        assert!(writer.write_player(&alice, []).is_err());
        writer
            .write_player(&bob, packed[1..].iter().map(Vec::as_slice))
            .unwrap();
        let meta = writer.finish().unwrap();
        assert_eq!(meta.games, 5);
        assert_eq!(meta.max_created_at, 30);

        let fixture = Fixture::open(&dir).unwrap();
        let bob_games = fixture.player(&bob).unwrap();
        assert_eq!(bob_games, PlayerGames { first: 3, count: 2 });
        assert_eq!(fixture.lower_bound(bob_games, 0), 3);
        assert_eq!(fixture.lower_bound(bob_games, 21), 4);
        assert_eq!(fixture.lower_bound(bob_games, 31), 5);
        let records = fixture.read_records(3, 5).unwrap();
        let read: Vec<FixtureGame> = record_bodies(&records)
            .map(|body| FixtureGame::unpack_body(body).unwrap())
            .collect();
        assert_eq!(read, games[1..]);
        assert!(
            fixture
                .player(&UserId::from_name("carol").unwrap())
                .is_none()
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
