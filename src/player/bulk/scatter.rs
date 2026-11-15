//! One pass over the parquet that writes the games of the population, as
//! the records the live indexer would write for them, to partitions by
//! user hash.
//!
//! ```text
//! partition := entry*
//! entry     := u32(len of the rest) UserHash u32(day) u32(row group)
//!              u32(rank among the rows of the day in the row group)
//!              u64(row) u16(plies) u8(flags) PlayerGame
//! flags     := 1 if the game is not indexed (a side without rating),
//!              then without PlayerGame
//! days      := (u32(day) u32(row group) u32(rows))*
//! names     := (UserHash string(name))*   (a name as seen in a game)
//! ```
//!
//! The month of the `PlayerGame` is a placeholder: it depends on
//! `createdAt`, which is only known after all row groups are counted
//! (rank of the row within its day, as for the mock fixture), and is set by
//! the build.

use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use shakmaty::{Color, Position, variant::Variant};

use crate::{
    mock_lila::{
        Noise,
        prepare::{day_of, fixture_game, name_hash},
        serve::{Presented, present},
        synth::{Synth, game_number, unit},
    },
    packed::{
        BorrowedLichessGame, GameId, Month, PackedMoves, PackedMovesDecoder, Rating, RatingMode,
        Speed, pack_string, unpack_string,
    },
    parquet::Input,
    player::{
        MAX_PLY, UserHash, UserHasher, UserId,
        bulk::{
            USER_HASH_KEY_FILE, for_each_row_group,
            population::{Member, Origin},
            save_user_hasher,
        },
        game::{BorrowedPlayerGame, PlayerGame},
        lila::{Classified, ExportGame},
    },
};

/// Placeholder month of scattered records.
pub fn placeholder_month() -> Month {
    Month::from_months_since_jan_1900(0)
}

/// Partitions by the leading bits of the user hash, so that partitions are
/// ordered like the keys.
#[derive(Debug, Clone, Copy)]
pub struct Partitions {
    pub bits: u32,
}

impl Partitions {
    pub fn count(self) -> usize {
        1 << self.bits
    }

    pub fn of(self, uh: UserHash) -> usize {
        (uh.to_u64() >> (64 - self.bits)) as usize
    }

    pub fn path(dir: &Path, p: usize) -> PathBuf {
        dir.join(format!("part_{p:05}.bin"))
    }
}

/// A scattered game of a player.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub uh: UserHash,
    pub day: u32,
    pub row_group: u32,
    pub rank: u32,
    pub row: u64,
    pub plies: u16,
    /// `None` for a game that is not indexed (but still received).
    pub game: Option<BorrowedPlayerGame<'a>>,
}

impl<'a> Entry<'a> {
    pub const HEADER: usize = 8 + 4 + 4 + 4 + 8 + 2 + 1;

    fn pack(&self, game: Option<&[u8]>, buf: &mut Vec<u8>) {
        let len = Self::HEADER + game.map_or(0, <[u8]>::len);
        buf.extend_from_slice(&(len as u32).to_le_bytes());
        buf.extend_from_slice(self.uh.as_bytes());
        buf.extend_from_slice(&self.day.to_le_bytes());
        buf.extend_from_slice(&self.row_group.to_le_bytes());
        buf.extend_from_slice(&self.rank.to_le_bytes());
        buf.extend_from_slice(&self.row.to_le_bytes());
        buf.extend_from_slice(&self.plies.to_le_bytes());
        buf.push(u8::from(game.is_none()));
        if let Some(game) = game {
            buf.extend_from_slice(game);
        }
    }

    /// Unpacks an entry (without the length prefix).
    pub fn unpack(b: &'a [u8]) -> Entry<'a> {
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"));
        Entry {
            uh: UserHash::from_bytes(b[..8].try_into().expect("8 bytes")),
            day: u32_at(8),
            row_group: u32_at(12),
            rank: u32_at(16),
            row: u64::from_le_bytes(b[20..28].try_into().expect("8 bytes")),
            plies: u16::from_le_bytes(b[28..30].try_into().expect("2 bytes")),
            game: (b[30] == 0).then(|| BorrowedPlayerGame::unpack(&b[Self::HEADER..])),
        }
    }
}

/// The entries (without length prefixes) of a partition file.
pub fn entries(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = bytes;
    std::iter::from_fn(move || {
        if rest.len() < 4 {
            return None;
        }
        let len = u32::from_le_bytes(rest[..4].try_into().expect("4 bytes")) as usize;
        let (entry, tail) = rest[4..].split_at(len);
        rest = tail;
        Some(entry)
    })
}

/// Whether the mock replaces a move of the game by an illegal one: such
/// games are converted via the export.
fn has_illegal_san(synth: &Synth, noise: &Noise, id: GameId) -> bool {
    unit(synth.seed, game_number(id), 4) < noise.illegal_san
}

/// The record of a standard game for the player of the given color,
/// directly from the parquet, with the plies of the whole game. Equal to
/// what the live indexer gets from the mock (checked by `verify`), except
/// for the month. `None` if the game is not indexed (a side without rating,
/// or noise of the mock: aborted, not started, opponent without account or
/// rating).
fn direct(
    id: GameId,
    game: &BorrowedLichessGame<'_>,
    color: Color,
    synth: &Synth,
    noise: &Noise,
) -> (Option<PlayerGame>, u16) {
    let rating = |color: Color| {
        game.players
            .get(color)
            .rating
            .and_then(|r| u16::try_from(i32::from(r)).ok())
            .filter(|&r| r > 0)
            .and_then(|r| Rating::try_from(i32::from(r)).ok())
    };
    let mut pos = game.initial_position();
    let mut moves = PackedMoves::encoder_with_capacity(MAX_PLY);
    let mut plies: u16 = 0;
    if let Ok(mut decoder) = PackedMovesDecoder::new(game.moves) {
        while let Ok(Some(m)) = decoder.decode_next(&pos) {
            if usize::from(plies) < MAX_PLY {
                moves.encode_next(&pos, m);
            }
            pos.play_unchecked(m);
            plies = plies.saturating_add(1);
        }
    }
    let (Some(player_rating), Some(opponent_rating)) = (rating(color), rating(!color)) else {
        return (None, plies);
    };
    let n = game_number(id);
    let seed = synth.seed;
    if unit(seed, n, 2) < noise.aborted
        || unit(seed, n, 3) < noise.no_start
        || unit(seed, n, 6) < noise.no_account
        || unit(seed, n, 7) < noise.no_rating
    {
        return (None, plies);
    }
    let me = game.players.get(color);
    let opponent = game.players.get(!color);
    let record = PlayerGame {
        id,
        speed: game.speed.unwrap_or(Speed::Blitz),
        mode: if synth.rated(game_number(id)) {
            RatingMode::Rated
        } else {
            RatingMode::Casual
        },
        month: placeholder_month(),
        outcome: game.outcome,
        color,
        player_rating,
        opponent_rating,
        opponent_name: opponent.name.to_owned(),
        opponent_title: opponent.title.map(str::to_owned),
        player_title: me.title.map(str::to_owned),
        setup: None,
        moves: moves.finish(),
    };
    (Some(record), plies)
}

/// The record as the live indexer gets it from the mock: the game as the
/// mock presents it, classified. Slow (SAN in between); for games with a
/// setup, and to verify `direct`.
fn via_export(
    id: GameId,
    game: &BorrowedLichessGame<'_>,
    user: &UserId,
    synth: &Synth,
    noise: &Noise,
) -> (Option<PlayerGame>, u16) {
    let fixture = fixture_game(id, game);
    let plies = u16::try_from(fixture.plies()).unwrap_or(u16::MAX);
    let synth = Synth { shift: 0, ..*synth };
    let Presented::Line(line) = present(&synth, noise, &fixture, user, u64::MAX, false) else {
        return (None, plies);
    };
    let export = ExportGame::parse(&line).expect("mock line");
    match export.classify(user) {
        Classified::Game { mut game, .. } => {
            game.month = placeholder_month();
            (Some(*game), plies)
        }
        Classified::Skip(_) => (None, plies),
    }
}

#[derive(Clone)]
pub struct ScatterConfig<'a> {
    pub input: &'a Input,
    pub members: &'a [Member],
    pub out: &'a Path,
    pub partitions: Partitions,
    pub threads: usize,
    pub synth: Synth,
    /// Noise of the mock (as it will serve the games).
    pub noise: Noise,
    /// Also convert every standard game via the export and compare.
    pub verify: bool,
}

#[derive(Debug, Default)]
pub struct ScatterStats {
    pub entries: AtomicU64,
    pub not_indexed: AtomicU64,
    pub via_export: AtomicU64,
    pub verified: AtomicU64,
    pub mismatches: AtomicU64,
    pub hash_collisions: AtomicU64,
    pub bytes: AtomicU64,
}

struct ThreadState {
    buffers: Vec<Vec<u8>>,
    /// Row counts per (day, row group).
    days: HashMap<(u32, u32), u32>,
    /// User id and name per member, when seen.
    users: HashMap<u32, (UserId, String, UserHash)>,
}

const FLUSH_BYTES: usize = 256 << 10;

/// Scatters the games of the members. Returns the statistics.
pub fn scatter(config: &ScatterConfig<'_>) -> io::Result<ScatterStats> {
    fs::create_dir_all(config.out)?;
    let hasher = UserHasher::generate();
    save_user_hasher(&hasher, &config.out.join(USER_HASH_KEY_FILE))?;
    let parts = config.partitions;
    let files: Vec<Mutex<File>> = (0..parts.count())
        .map(|p| File::create(Partitions::path(config.out, p)).map(Mutex::new))
        .collect::<io::Result<_>>()?;
    let lookup: HashMap<u64, u32> = config
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name_hash, i as u32))
        .collect();
    let stats = ScatterStats::default();
    let write_error: Mutex<Option<io::Error>> = Mutex::new(None);
    let flush = |p: usize, buf: &mut Vec<u8>| {
        stats.bytes.fetch_add(buf.len() as u64, Ordering::Relaxed);
        if let Err(err) = files[p].lock().expect("partition").write_all(buf) {
            *write_error.lock().expect("error") = Some(err);
        }
        buf.clear();
    };

    let states = for_each_row_group(
        config.input,
        config.threads,
        || ThreadState {
            buffers: vec![Vec::new(); parts.count()],
            days: HashMap::new(),
            users: HashMap::new(),
        },
        |state, rg, batch| {
            let first = config.input.first_row(rg);
            let mut record = Vec::new();
            for (i, (id, game)) in batch.rows().enumerate() {
                let row = first + i as u64;
                let day = day_of(game.date);
                let count = state.days.entry((day, rg as u32)).or_insert(0);
                let rank = *count;
                *count += 1;
                for color in Color::ALL {
                    let name = game.players.get(color).name;
                    let Some(&member) = lookup.get(&name_hash(name.as_bytes())) else {
                        continue;
                    };
                    let (user, _, uh) = state.users.entry(member).or_insert_with(|| {
                        let user = UserId::from_name(name).expect("valid user name");
                        let uh = hasher.hash(&user);
                        (user, name.to_owned(), uh)
                    });
                    let uh = *uh;
                    if !user.is_name(name) {
                        stats.hash_collisions.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let standard = game.setup.is_none()
                        && game.initial_position().variant() == Variant::Chess
                        && !has_illegal_san(&config.synth, &config.noise, id);
                    let (converted, plies) = if standard {
                        let direct = direct(id, &game, color, &config.synth, &config.noise);
                        if config.verify {
                            let exact = via_export(id, &game, user, &config.synth, &config.noise);
                            stats.verified.fetch_add(1, Ordering::Relaxed);
                            if (exact.0 != direct.0 || exact.1 != direct.1)
                                && stats.mismatches.fetch_add(1, Ordering::Relaxed) < 10
                            {
                                log::error!(
                                    "mismatch for {id} of {user}: direct {direct:?}, via export {exact:?}"
                                );
                            }
                        }
                        direct
                    } else {
                        stats.via_export.fetch_add(1, Ordering::Relaxed);
                        via_export(id, &game, user, &config.synth, &config.noise)
                    };
                    record.clear();
                    if let Some(converted) = &converted {
                        converted.pack(&mut record);
                    } else {
                        stats.not_indexed.fetch_add(1, Ordering::Relaxed);
                    }
                    let p = parts.of(uh);
                    Entry {
                        uh,
                        day,
                        row_group: rg as u32,
                        rank,
                        row,
                        plies,
                        game: None,
                    }
                    .pack(
                        converted.is_some().then_some(&record[..]),
                        &mut state.buffers[p],
                    );
                    stats.entries.fetch_add(1, Ordering::Relaxed);
                    if state.buffers[p].len() >= FLUSH_BYTES {
                        flush(p, &mut state.buffers[p]);
                    }
                }
            }
        },
    )?;

    let mut days: HashMap<(u32, u32), u32> = HashMap::new();
    let mut names: HashMap<UserHash, String> = HashMap::new();
    for mut state in states {
        for (p, buf) in state.buffers.iter_mut().enumerate() {
            if !buf.is_empty() {
                flush(p, buf);
            }
        }
        for (k, v) in state.days {
            *days.entry(k).or_default() += v;
        }
        for (_, (_, name, uh)) in state.users {
            names.entry(uh).or_insert(name);
        }
    }
    if let Some(err) = write_error.into_inner().expect("error") {
        return Err(err);
    }

    let mut days: Vec<_> = days.into_iter().collect();
    days.sort_unstable();
    let mut w = BufWriter::new(File::create(config.out.join("days.bin"))?);
    for ((day, rg), rows) in days {
        w.write_all(&day.to_le_bytes())?;
        w.write_all(&rg.to_le_bytes())?;
        w.write_all(&rows.to_le_bytes())?;
    }
    w.flush()?;

    let mut names: Vec<_> = names.into_iter().collect();
    names.sort_unstable();
    let mut w = BufWriter::new(File::create(config.out.join("names.bin"))?);
    let mut buf = Vec::new();
    for (uh, name) in &names {
        buf.clear();
        buf.extend_from_slice(uh.as_bytes());
        pack_string(&mut buf, name);
        w.write_all(&buf)?;
    }
    w.flush()?;

    let requests = config
        .members
        .iter()
        .filter(|m| m.origin == Origin::Requests)
        .count();
    log::info!(
        "scattered {} entries ({} not indexed, {} via export, {} hash collisions), {} bytes, {} of {} players seen ({requests} requests players)",
        stats.entries.load(Ordering::Relaxed),
        stats.not_indexed.load(Ordering::Relaxed),
        stats.via_export.load(Ordering::Relaxed),
        stats.hash_collisions.load(Ordering::Relaxed),
        stats.bytes.load(Ordering::Relaxed),
        names.len(),
        config.members.len(),
    );
    if config.verify {
        log::info!(
            "verified {} standard games against the export path: {} mismatches",
            stats.verified.load(Ordering::Relaxed),
            stats.mismatches.load(Ordering::Relaxed)
        );
    }
    Ok(stats)
}

/// Rows per (day, row group) of the input, as written by `scatter`.
pub fn read_days(dir: &Path) -> io::Result<Vec<((u32, u32), u32)>> {
    let mut bytes = Vec::new();
    BufReader::new(File::open(dir.join("days.bin"))?).read_to_end(&mut bytes)?;
    let u32_at = |c: &[u8], i: usize| u32::from_le_bytes(c[i..i + 4].try_into().expect("4 bytes"));
    Ok(bytes
        .as_chunks::<12>()
        .0
        .iter()
        .map(|c| ((u32_at(c, 0), u32_at(c, 4)), u32_at(c, 8)))
        .collect())
}

/// Names of the players, as written by `scatter`.
pub fn read_names(dir: &Path) -> io::Result<HashMap<UserHash, String>> {
    let mut bytes = Vec::new();
    BufReader::new(File::open(dir.join("names.bin"))?).read_to_end(&mut bytes)?;
    let mut buf = &bytes[..];
    let mut names = HashMap::new();
    while !buf.is_empty() {
        let uh = UserHash::from_bytes(buf[..8].try_into().expect("8 bytes"));
        buf = &buf[8..];
        let name = unpack_string(&mut buf).to_owned();
        names.insert(uh, name);
    }
    Ok(names)
}
