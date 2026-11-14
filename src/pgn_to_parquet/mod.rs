mod dedup;
mod pgn;

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    fs::File,
    io::{self, BufReader, BufWriter, Read},
    mem,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use bytes::Bytes;
use crossbeam::channel;
pub use dedup::RecentGameIds;
use indicatif::{
    HumanBytes, HumanCount, MultiProgress, MultiProgressAlignment, ProgressBar, ProgressStyle,
};
use parquet::{
    basic::{Compression, ZstdLevel},
    column::writer::ColumnCloseResult,
    file::{
        metadata::{PageIndexPolicy, ParquetMetaDataReader},
        properties::WriterProperties,
        reader::{FileReader, SerializedFileReader},
        writer::SerializedFileWriter,
    },
    schema::types::ColumnPath,
};
pub use pgn::GameVisitor;
use pgn_reader::Reader as PgnReader;

use crate::{lila_variant::LilaVariant, packed::GameId, parquet::GameColumns};

const TYPICAL_GAME_BYTES: usize = 2500;

pub struct Config {
    pub threads: usize,
    pub readers: usize,
    pub games_per_chunk: usize,
    pub memory_budget: u64,
    pub read_buffer: usize,
    /// Zstd level for meta data columns, or 0 for no compression.
    pub zstd_level: i32,
    /// Zstd level for the moves column, or 0 for no compression.
    pub moves_zstd_level: i32,
    /// Keep only the first moves of each game (`None` for all).
    pub max_ply: Option<usize>,
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub games: u64,
    pub unplayed: u64,
    pub unfinished: u64,
    pub bots: u64,
    pub unrated_players: u64,
    pub duplicates: u64,
    pub invalid: u64,
}

impl Stats {
    fn add(&mut self, other: &Stats) {
        self.games += other.games;
        self.unplayed += other.unplayed;
        self.unfinished += other.unfinished;
        self.bots += other.bots;
        self.unrated_players += other.unrated_players;
        self.duplicates += other.duplicates;
        self.invalid += other.invalid;
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct FileId(usize);

impl FileId {
    fn index(self) -> usize {
        self.0
    }
}

/// Bounds the memory used by chunks that are waiting to be parsed or
/// appended.
///
/// Chunks of the oldest file that is not yet completely appended are exempt,
/// so that progress is always possible.
struct MemoryBudget {
    state: Mutex<BudgetState>,
    changed: Condvar,
    limit: u64,
}

struct BudgetState {
    used: u64,
    head_file: FileId,
    cancelled: bool,
}

impl MemoryBudget {
    fn new(limit: u64) -> MemoryBudget {
        MemoryBudget {
            state: Mutex::new(BudgetState {
                used: 0,
                head_file: FileId::default(),
                cancelled: false,
            }),
            changed: Condvar::new(),
            limit,
        }
    }

    fn lock(&self) -> MutexGuard<'_, BudgetState> {
        self.state.lock().expect("lock budget")
    }

    fn acquire(&self, bytes: u64, file: FileId) -> Result<(), Cancelled> {
        let mut state = self.lock();
        while state.used + bytes > self.limit && file != state.head_file && !state.cancelled {
            state = self.changed.wait(state).expect("wait budget");
        }
        if state.cancelled {
            return Err(Cancelled);
        }
        state.used += bytes;
        Ok(())
    }

    fn release(&self, bytes: u64) {
        self.lock().used -= bytes;
        self.changed.notify_all();
    }

    /// Replaces a reservation without waiting, for a chunk that changed size.
    fn swap(&self, old: u64, new: u64) {
        let mut state = self.lock();
        state.used = state.used + new - old;
        drop(state);
        self.changed.notify_all();
    }

    fn set_head_file(&self, file: FileId) {
        self.lock().head_file = file;
        self.changed.notify_all();
    }

    fn used(&self) -> u64 {
        self.lock().used
    }

    fn cancel(&self) {
        self.lock().cancelled = true;
        self.changed.notify_all();
    }
}

struct Cancelled;

enum ReadError {
    Io(io::Error),
    Cancelled,
}

impl From<io::Error> for ReadError {
    fn from(err: io::Error) -> ReadError {
        ReadError::Io(err)
    }
}

impl From<Cancelled> for ReadError {
    fn from(Cancelled: Cancelled) -> ReadError {
        ReadError::Cancelled
    }
}

struct CancelOnDrop<'a>(&'a MemoryBudget);

impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct Work {
    file: FileId,
    seq: usize,
    chunk: PgnChunk,
}

struct Encoded {
    file: FileId,
    seq: usize,
    /// Encoded parquet with a single row group.
    parquet: Bytes,
    /// For deduplication.
    game_ids: Vec<(LilaVariant, GameId)>,
    /// Games the parser found, and games the splitter found.
    parsed: usize,
    expected: usize,
    stats: Stats,
    errors: Vec<String>,
}

enum Message {
    Encoded(Encoded),
    FileRead { file: FileId, chunks: usize },
    Failed(io::Error),
}

struct ProgressRead<R> {
    inner: R,
    file_bar: ProgressBar,
    total_bar: ProgressBar,
}

impl<R: Read> Read for ProgressRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.file_bar.inc(n as u64);
        self.total_bar.inc(n as u64);
        Ok(n)
    }
}

fn open_pgn(
    path: &Path,
    read_buffer: usize,
    file_bar: ProgressBar,
    total_bar: ProgressBar,
) -> io::Result<Box<dyn Read>> {
    let file = ProgressRead {
        inner: File::open(path)?,
        file_bar,
        total_bar,
    };
    let buffered = BufReader::with_capacity(read_buffer, file);
    Ok(if path.extension() == Some(OsStr::new("zst")) {
        Box::new(zstd::Decoder::with_buffer(buffered)?)
    } else {
        Box::new(buffered)
    })
}

fn compression(zstd_level: i32) -> Compression {
    if zstd_level == 0 {
        Compression::UNCOMPRESSED
    } else {
        Compression::ZSTD(ZstdLevel::try_new(zstd_level).expect("valid zstd level"))
    }
}

pub fn writer_properties(zstd_level: i32, moves_zstd_level: i32) -> Arc<WriterProperties> {
    Arc::new(
        GameColumns::properties()
            .set_compression(compression(zstd_level))
            .set_column_compression(ColumnPath::from("moves"), compression(moves_zstd_level))
            .build(),
    )
}

fn encode_chunk(
    work: &Work,
    columns: &mut GameColumns,
    properties: &Arc<WriterProperties>,
    max_ply: Option<usize>,
) -> io::Result<Encoded> {
    let mut stats = Stats::default();
    let mut errors = Vec::new();
    let mut game_ids = Vec::with_capacity(work.chunk.games);
    let mut parsed = 0;
    columns.clear();

    let mut reader = PgnReader::new(&work.chunk.bytes[..]);
    let mut visitor = GameVisitor {
        max_ply: max_ply.unwrap_or(usize::MAX),
    };
    for game in reader.read_games(&mut visitor) {
        parsed += 1;
        let game = match game? {
            Ok(game) => game,
            Err(err) => {
                stats.invalid += 1;
                errors.push(err.to_string());
                continue;
            }
        };

        if game.plies() < 2 {
            stats.unplayed += 1;
            continue;
        }

        if game.outcome().is_unknown() {
            stats.unfinished += 1;
            continue;
        }

        if game.has_bot() {
            stats.bots += 1;
            continue;
        }

        if !game.has_ratings() {
            stats.unrated_players += 1;
            continue;
        }

        let variant = game.lila_variant();
        match game.into_game() {
            Ok((game_id, game)) => {
                game_ids.push((variant, game_id));
                columns.push(game_id, game);
                stats.games += 1;
            }
            Err(err) => {
                stats.invalid += 1;
                errors.push(err.to_string());
            }
        }
    }

    let mut writer =
        SerializedFileWriter::new(Vec::new(), GameColumns::schema(), Arc::clone(properties))?;
    if !columns.is_empty() {
        columns.write_row_group(&mut writer)?;
    }
    let parquet = Bytes::from(writer.into_inner()?);

    Ok(Encoded {
        file: work.file,
        seq: work.seq,
        parquet,
        game_ids,
        parsed,
        expected: work.chunk.games,
        stats,
        errors,
    })
}

/// Appends the single row group of `parquet`, without the rows that are not
/// kept.
pub fn append_row_group(
    writer: &mut SerializedFileWriter<BufWriter<File>>,
    parquet: &Bytes,
    keep: &[bool],
) -> parquet::errors::Result<()> {
    let kept = keep.iter().filter(|&&k| k).count();
    if kept == 0 {
        return Ok(());
    }

    let metadata = ParquetMetaDataReader::new()
        .with_page_index_policy(PageIndexPolicy::Optional)
        .parse_and_finish(parquet)?;
    let page_index = metadata.page_index();
    assert_eq!(metadata.num_row_groups(), 1);
    let rg = metadata.row_group(0);
    assert_eq!(rg.num_rows() as usize, keep.len());

    if kept == keep.len() {
        // Copy the row group byte for byte.
        let mut rg_out = writer.next_row_group()?;
        for (col_idx, column) in rg.columns().iter().enumerate() {
            rg_out.append_column(
                parquet,
                ColumnCloseResult {
                    bytes_written: column.compressed_size() as u64,
                    rows_written: rg.num_rows() as u64,
                    metadata: column.clone(),
                    bloom_filter: None,
                    column_index: page_index
                        .and_then(|pi| pi.column_index(0, col_idx))
                        .cloned(),
                    offset_index: page_index
                        .and_then(|pi| pi.offset_index(0, col_idx))
                        .cloned(),
                },
            )?;
        }
        rg_out.close()?;
    } else {
        // Rewrite the row group without the dropped rows.
        let reader = SerializedFileReader::new(parquet.clone())?;
        let mut columns = GameColumns::default();
        columns.read_columns(&*reader.get_row_group(0)?)?;
        let mut filtered = GameColumns::default();
        let mut keep = keep.iter();
        columns.scatter(
            |_| keep.next().expect("keep flag per row").then_some(0),
            &mut [&mut filtered],
        );
        assert_eq!(filtered.len(), kept);
        filtered.write_row_group(writer)?;
    }

    Ok(())
}

pub fn convert(
    pgns: &[PathBuf],
    out: &Path,
    config: &Config,
    progress: &MultiProgress,
) -> io::Result<Stats> {
    let started = Instant::now();
    let readers = config.readers.clamp(1, pgns.len().max(1));
    let workers = config.threads.saturating_sub(readers).max(1);
    let properties = writer_properties(config.zstd_level, config.moves_zstd_level);

    let total_bytes = pgns
        .iter()
        .map(|path| path.metadata().map(|m| m.len()))
        .sum::<io::Result<u64>>()?;

    progress.set_alignment(MultiProgressAlignment::Bottom);
    let total_bar = progress.add(
        ProgressBar::new(total_bytes).with_style(
            ProgressStyle::with_template(
                "{spinner} {elapsed_precise} {wide_bar} {bytes:>11}/{total_bytes:>11} {bytes_per_sec:>12} {eta:>6} {msg}",
            )
            .expect("progress template"),
        ),
    );
    total_bar.enable_steady_tick(Duration::from_millis(250));
    let file_style = ProgressStyle::with_template(
        "{spinner} {prefix} {wide_bar} {bytes:>11}/{total_bytes:>11} {bytes_per_sec:>12} {eta:>6}",
    )
    .expect("progress template");

    let budget = MemoryBudget::new(config.memory_budget);
    let next_file = AtomicUsize::new(0);
    let games_written = AtomicU64::new(0);
    let (work_tx, work_rx) = channel::bounded::<Work>(2 * readers);
    let (msg_tx, msg_rx) = channel::unbounded::<Message>();

    // Pool of large buffers, typically ~200 MiB each.
    let largest_chunk = AtomicUsize::new(config.games_per_chunk * TYPICAL_GAME_BYTES);
    let (pool_tx, pool_rx) = channel::unbounded::<Vec<u8>>();
    let take_buffer = || {
        pool_rx.try_recv().unwrap_or_else(|_| {
            Vec::with_capacity(largest_chunk.load(Ordering::Relaxed) + (4 << 20))
        })
    };

    let mut writer = SerializedFileWriter::new(
        BufWriter::with_capacity(4 << 20, File::create(out)?),
        GameColumns::schema(),
        Arc::clone(&properties),
    )?;

    let total = thread::scope(|s| {
        for _ in 0..readers {
            let work_tx = work_tx.clone();
            let msg_tx = msg_tx.clone();
            let pool_tx = pool_tx.clone();
            let take_buffer = &take_buffer;
            let largest_chunk = &largest_chunk;
            let budget = &budget;
            let next_file = &next_file;
            let total_bar = &total_bar;
            let file_style = &file_style;
            thread::Builder::new()
                .name("reader".into())
                .spawn_scoped(s, move || {
                    loop {
                        let file = FileId(next_file.fetch_add(1, Ordering::Relaxed));
                        let Some(path) = pgns.get(file.index()) else {
                            break;
                        };

                        let file_bar = progress.insert_before(
                            total_bar,
                            ProgressBar::new(path.metadata().map_or(0, |m| m.len()))
                                .with_style(file_style.clone())
                                .with_prefix(path.display().to_string()),
                        );

                        let mut chunks = 0;
                        let result = (|| -> Result<(), ReadError> {
                            let reader = open_pgn(
                                path,
                                config.read_buffer,
                                file_bar.clone(),
                                total_bar.clone(),
                            )?;
                            let mut splitter = PgnSplitter::new(reader, take_buffer());
                            while let Some(chunk) =
                                splitter.next_chunk(config.games_per_chunk, take_buffer())?
                            {
                                largest_chunk.fetch_max(chunk.bytes.len(), Ordering::Relaxed);
                                budget.acquire(chunk.bytes.len() as u64, file)?;
                                let work = Work {
                                    file,
                                    seq: chunks,
                                    chunk,
                                };
                                work_tx.send(work).map_err(|_| Cancelled)?;
                                chunks += 1;
                            }
                            pool_tx
                                .send(splitter.into_buffer())
                                .expect("recycle buffer");
                            Ok(())
                        })();

                        file_bar.finish_and_clear();
                        progress.remove(&file_bar);

                        let msg = match result {
                            Ok(()) => Message::FileRead { file, chunks },
                            Err(ReadError::Cancelled) => break,
                            Err(ReadError::Io(err)) => Message::Failed(io::Error::new(
                                err.kind(),
                                format!("{}: {err}", path.display()),
                            )),
                        };
                        if msg_tx.send(msg).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawn reader");
        }
        drop(work_tx);

        for _ in 0..workers {
            let work_rx = work_rx.clone();
            let msg_tx = msg_tx.clone();
            let pool_tx = pool_tx.clone();
            let budget = &budget;
            let properties = &properties;
            thread::Builder::new()
                .name("worker".into())
                .spawn_scoped(s, move || {
                    let mut columns = GameColumns::default();
                    for work in work_rx {
                        let raw_len = work.chunk.bytes.len() as u64;
                        let result = encode_chunk(&work, &mut columns, properties, config.max_ply);
                        pool_tx.send(work.chunk.bytes).expect("recycle buffer");
                        let msg = match result {
                            Ok(encoded) => {
                                budget.swap(raw_len, encoded.parquet.len() as u64);
                                Message::Encoded(encoded)
                            }
                            Err(err) => Message::Failed(io::Error::new(
                                err.kind(),
                                format!("encoding chunk: {err}"),
                            )),
                        };
                        if msg_tx.send(msg).is_err() {
                            break; // Cancelled
                        }
                    }
                })
                .expect("spawn worker");
        }
        drop(msg_tx);

        // Unblock and stop readers and workers when returning early. Dropping
        // msg_rx also stops workers, and then readers.
        let msg_rx = msg_rx;
        let _cancel = CancelOnDrop(&budget);

        // Append encoded chunks in order.
        let mut pending: BTreeMap<(FileId, usize), Encoded> = BTreeMap::new();
        let mut file_chunks: HashMap<FileId, usize> = HashMap::new();
        let mut next = (FileId::default(), 0);
        // Per variant, because files are ordered by month across variants,
        // while duplicates are between consecutive months of a variant.
        let mut recent_game_ids: HashMap<LilaVariant, RecentGameIds> = HashMap::new();
        let mut file_stats = Stats::default();
        let mut file_started = Instant::now();
        let mut total = Stats::default();
        let mut keep = Vec::new();

        while next.0.index() < pgns.len() {
            match msg_rx.recv().expect("readers and workers alive") {
                Message::Encoded(encoded) => {
                    pending.insert((encoded.file, encoded.seq), encoded);
                }
                Message::FileRead { file, chunks } => {
                    file_chunks.insert(file, chunks);
                }
                Message::Failed(err) => return Err(err),
            }

            loop {
                if let Some(encoded) = pending.remove(&next) {
                    assert_eq!(
                        encoded.parsed,
                        encoded.expected,
                        "{} chunk {}: parser and splitter disagree on number of games",
                        pgns[encoded.file.index()].display(),
                        encoded.seq,
                    );
                    for err in &encoded.errors {
                        let _ = progress.println(format!("skipping game: {err}"));
                    }

                    keep.clear();
                    keep.extend(encoded.game_ids.iter().map(|&(variant, game_id)| {
                        recent_game_ids.entry(variant).or_default().insert(game_id)
                    }));
                    let kept = keep.iter().filter(|&&k| k).count() as u64;
                    file_stats.add(&Stats {
                        duplicates: encoded.stats.games - kept,
                        games: kept,
                        ..encoded.stats
                    });

                    append_row_group(&mut writer, &encoded.parquet, &keep).map_err(|err| {
                        io::Error::other(format!("writing {}: {err}", out.display()))
                    })?;
                    budget.release(encoded.parquet.len() as u64);

                    let games = games_written.fetch_add(kept, Ordering::Relaxed) + kept;
                    total_bar.set_message(format!(
                        "{} games, {}/s, {} in memory",
                        HumanCount(games),
                        HumanCount((games as f64 / started.elapsed().as_secs_f64()) as u64),
                        HumanBytes(budget.used())
                    ));

                    next.1 += 1;
                } else if file_chunks.get(&next.0) == Some(&next.1) {
                    let file = next.0;
                    total.add(&file_stats);
                    let _ = progress.println(format!(
                            "[{:>8.1?}] {}/{} {}: {} games, {} unplayed, {} unfinished, {} bots, {} without ratings, {} duplicates, {} invalid, {:.1?}",
                            started.elapsed(),
                            file.index() + 1,
                            pgns.len(),
                            pgns[file.index()].display(),
                            file_stats.games,
                            file_stats.unplayed,
                            file_stats.unfinished,
                            file_stats.bots,
                            file_stats.unrated_players,
                            file_stats.duplicates,
                            file_stats.invalid,
                            file_started.elapsed(),
                        ));
                    file_stats = Stats::default();
                    file_started = Instant::now();
                    next = (FileId(file.index() + 1), 0);
                    budget.set_head_file(next.0);
                } else {
                    break;
                }
            }
        }

        Ok(total)
    });

    total_bar.finish_and_clear();
    let total = total?;
    writer.close()?;
    Ok(total)
}

/// Copies everything a reader delivers also into a buffer.
struct Tee<R> {
    inner: R,
    delivered: Vec<u8>,
}

impl<R: Read> Read for Tee<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.delivered.extend_from_slice(&buf[..n]);
        Ok(n)
    }
}

impl<R> Tee<R> {
    fn new(inner: R, mut recycled_buffer: Vec<u8>) -> Self {
        recycled_buffer.clear();
        Tee {
            inner,
            delivered: recycled_buffer,
        }
    }

    fn take_delivered(&mut self, unconsumed: usize, mut recycled_buffer: Vec<u8>) -> Vec<u8> {
        let cut = self.delivered.len() - unconsumed;
        recycled_buffer.clear();
        recycled_buffer.extend_from_slice(&self.delivered[cut..]);
        self.delivered.truncate(cut);
        mem::replace(&mut self.delivered, recycled_buffer)
    }

    fn into_delivered(self) -> Vec<u8> {
        self.delivered
    }
}

/// Splits a PGN stream into chunks of consecutive games.
struct PgnSplitter<R> {
    reader: PgnReader<Tee<R>>,
    done: bool,
}

impl<R: Read> PgnSplitter<R> {
    pub fn new(reader: R, recycled_buffer: Vec<u8>) -> Self {
        PgnSplitter {
            reader: PgnReader::build(Tee::new(reader, recycled_buffer))
                .with_minimum_buffer_capacity(64 << 10)
                .finish(),
            done: false,
        }
    }

    pub fn next_chunk(
        &mut self,
        max_games: usize,
        recycled_buffer: Vec<u8>,
    ) -> io::Result<Option<PgnChunk>> {
        if self.done {
            return Ok(None);
        }

        let mut games = 0;
        while games < max_games {
            if !self.reader.skip_game()? {
                self.done = true;
                break;
            }
            games += 1;
        }

        if games == 0 {
            return Ok(None);
        }

        let unconsumed = self.reader.buffer().len();
        Ok(Some(PgnChunk {
            bytes: self
                .reader
                .get_mut()
                .take_delivered(unconsumed, recycled_buffer),
            games,
        }))
    }

    pub fn into_buffer(self) -> Vec<u8> {
        self.reader.into_inner().into_delivered()
    }
}

/// A run of consecutive games of a PGN stream, starting at a game boundary.
pub struct PgnChunk {
    bytes: Vec<u8>,
    games: usize,
}
