mod id;
mod pgn;

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufWriter},
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::Duration,
};

use crossbeam::channel;
pub use id::push_uci;
use indicatif::{HumanCount, ProgressBar, ProgressStyle};
use parquet::{
    basic::{Compression, ZstdLevel},
    file::writer::SerializedFileWriter,
};
pub use pgn::MastersVisitor;
use pgn_reader::Reader as PgnReader;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    packed::{GameId, LichessGame},
    parquet::GameColumns,
};

pub struct Config {
    pub threads: usize,
    pub row_group_len: usize,
    /// Zstd level for meta data columns, or 0 for no compression.
    pub zstd_level: i32,
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub games: u64,
    /// Games whose deterministic id was replaced by a legacy id.
    pub legacy: u64,
    pub unplayed: u64,
    pub duplicates: u64,
    pub invalid: u64,
}

/// Reads `legacy_ids.csv` of the masters database, mapping deterministic
/// ids to the ids assigned by the first version of the database. For
/// deterministic ids with multiple rows (duplicate games), the first row
/// is the one that survived in production.
pub fn read_legacy_ids(path: &Path) -> io::Result<FxHashMap<GameId, GameId>> {
    let invalid = |msg: String| io::Error::new(io::ErrorKind::InvalidData, msg);
    let mut legacy_ids = FxHashMap::default();
    for (i, line) in fs::read_to_string(path)?.lines().enumerate() {
        if i == 0 && line == "deterministic,legacy" {
            continue;
        }
        let (deterministic, legacy) = line.split_once(',').ok_or_else(|| {
            invalid(format!(
                "{}:{}: expected two columns",
                path.display(),
                i + 1
            ))
        })?;
        let deterministic: GameId = deterministic
            .parse()
            .map_err(|err| invalid(format!("{}:{}: {err}", path.display(), i + 1)))?;
        let legacy: GameId = legacy
            .parse()
            .map_err(|err| invalid(format!("{}:{}: {err}", path.display(), i + 1)))?;
        legacy_ids.entry(deterministic).or_insert(legacy);
    }
    Ok(legacy_ids)
}

/// Collects PGN files from paths. Directories are walked recursively, and
/// their `.pgn` files sorted by path, which is chronological for the
/// `games/yyyy/mm/dd.pgn` layout of the masters database. Unknown months and
/// days (`xx`) come first, because games are indexed under the earliest
/// possible month, and game indexes (rows) must be chronological by month.
pub fn collect_pgns(paths: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<io::Result<_>>()?;
        entries.sort_by_key(|path| {
            let unknown = path
                .file_stem()
                .is_some_and(|stem| stem.as_encoded_bytes() == b"xx");
            (!unknown, path.clone())
        });
        for path in entries {
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.extension().is_some_and(|ext| ext == "pgn") {
                out.push(path);
            }
        }
        Ok(())
    }

    let mut pgns = Vec::new();
    for path in paths {
        if path.is_dir() {
            walk(path, &mut pgns)?;
        } else {
            pgns.push(path.clone());
        }
    }
    Ok(pgns)
}

struct Parsed {
    games: Vec<(GameId, LichessGame)>,
    stats: Stats,
    errors: Vec<String>,
}

fn parse_file(path: &Path) -> io::Result<Parsed> {
    let bytes = fs::read(path)?;
    let mut parsed = Parsed {
        games: Vec::new(),
        stats: Stats::default(),
        errors: Vec::new(),
    };
    let mut reader = PgnReader::new(&bytes[..]);
    for game in reader.read_games(&mut MastersVisitor) {
        let game = match game? {
            Ok(game) => game,
            Err(err) => {
                parsed.stats.invalid += 1;
                parsed.errors.push(err.to_string());
                continue;
            }
        };
        if game.plies() < 2 {
            parsed.stats.unplayed += 1;
            continue;
        }
        match game.into_game() {
            Ok(game) => {
                parsed.games.push(game);
                parsed.stats.games += 1;
            }
            Err(err) => {
                parsed.stats.invalid += 1;
                parsed.errors.push(err.to_string());
            }
        }
    }
    Ok(parsed)
}

fn compression(zstd_level: i32) -> Compression {
    if zstd_level == 0 {
        Compression::UNCOMPRESSED
    } else {
        Compression::ZSTD(ZstdLevel::try_new(zstd_level).expect("valid zstd level"))
    }
}

pub fn convert(
    pgns: &[PathBuf],
    legacy_ids: &FxHashMap<GameId, GameId>,
    out: &Path,
    config: &Config,
) -> io::Result<Stats> {
    let total_bytes = pgns
        .iter()
        .map(|path| path.metadata().map(|m| m.len()))
        .sum::<io::Result<u64>>()?;
    let progress = ProgressBar::new(total_bytes).with_style(
        ProgressStyle::with_template(
            "{spinner} {elapsed_precise} {wide_bar} {bytes:>11}/{total_bytes:>11} {bytes_per_sec:>12} {eta:>6} {msg}",
        )
        .expect("progress template"),
    );
    progress.enable_steady_tick(Duration::from_millis(250));

    let properties = Arc::new(
        GameColumns::properties()
            .set_compression(compression(config.zstd_level))
            .build(),
    );
    let mut writer = SerializedFileWriter::new(
        BufWriter::with_capacity(4 << 20, File::create(out)?),
        GameColumns::schema(),
        properties,
    )?;

    let workers = config.threads.max(1);
    let (work_tx, work_rx) = channel::bounded::<usize>(workers);
    let (done_tx, done_rx) = channel::unbounded::<(usize, io::Result<Parsed>)>();

    let total = thread::scope(|s| -> io::Result<Stats> {
        for _ in 0..workers {
            let work_rx = work_rx.clone();
            let done_tx = done_tx.clone();
            s.spawn(move || {
                for file in work_rx {
                    let result = parse_file(&pgns[file]).map_err(|err| {
                        io::Error::new(err.kind(), format!("{}: {err}", pgns[file].display()))
                    });
                    if done_tx.send((file, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(done_tx);

        // Feed files in order. The bounded channel keeps the number of parsed
        // files waiting to be appended small.
        s.spawn(move || {
            for file in 0..pgns.len() {
                if work_tx.send(file).is_err() {
                    break;
                }
            }
        });

        let mut pending: BTreeMap<usize, Parsed> = BTreeMap::new();
        let mut next = 0;
        let mut seen: FxHashSet<GameId> = FxHashSet::default();
        let mut columns = GameColumns::default();
        let mut total = Stats::default();

        while next < pgns.len() {
            let (file, result) = done_rx.recv().expect("workers alive");
            pending.insert(file, result?);

            while let Some(parsed) = pending.remove(&next) {
                for err in &parsed.errors {
                    progress
                        .suspend(|| eprintln!("{}: skipping game: {err}", pgns[next].display()));
                }
                total.unplayed += parsed.stats.unplayed;
                total.invalid += parsed.stats.invalid;

                for (deterministic_id, game) in parsed.games {
                    let game_id = match legacy_ids.get(&deterministic_id) {
                        Some(&legacy_id) => {
                            total.legacy += 1;
                            legacy_id
                        }
                        None => deterministic_id,
                    };
                    if !seen.insert(game_id) {
                        total.duplicates += 1;
                        progress.suspend(|| {
                            eprintln!(
                                "{}: skipping duplicate {} ({} - {}, {})",
                                pgns[next].display(),
                                game_id,
                                game.players.white.name,
                                game.players.black.name,
                                game.event.as_ref().expect("masters event").event,
                            )
                        });
                        continue;
                    }
                    total.games += 1;
                    columns.push(game_id, game);
                }

                if columns.len() >= config.row_group_len {
                    columns.write_row_group(&mut writer)?;
                    columns.clear();
                }

                progress.inc(pgns[next].metadata().map_or(0, |m| m.len()));
                progress.set_message(format!("{} games", HumanCount(total.games)));
                next += 1;
            }
        }

        if !columns.is_empty() {
            columns.write_row_group(&mut writer)?;
        }
        Ok(total)
    })?;

    progress.finish_and_clear();
    writer.close()?;
    Ok(total)
}
