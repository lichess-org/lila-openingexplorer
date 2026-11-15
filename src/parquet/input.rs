use std::{
    fs::File,
    io,
    os::unix::fs::{FileExt, OpenOptionsExt},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
};

use crossbeam::channel;

use crate::{
    parquet::{GameBatch, GameReader, PositionalFile, RowGroupBytes},
    util::{AlignedBuf, DIRECT_ALIGN, timed},
};

/// The row groups of all input files, in order.
pub struct Input {
    readers: Vec<GameReader<PositionalFile>>,
    /// `O_DIRECT` handles of the input files, for fetching row groups
    /// without the page cache.
    direct_files: Option<Vec<File>>,
    /// Pairs of reader index and row group index within that reader.
    row_groups: Vec<(usize, usize)>,
    /// Index of the first row of each row group across the selected row
    /// groups, and the total number of selected rows at the end.
    row_offsets: Vec<u64>,
    /// Rows of the skipped row groups.
    skipped_rows: u64,
    /// Rows of all row groups of all inputs, selected or not.
    all_rows: u64,
}

impl Input {
    /// Opens the parquet files, using only `limit` row groups after skipping
    /// the first `skip` row groups of the concatenated inputs.
    pub fn open(
        paths: &[PathBuf],
        skip: usize,
        limit: Option<usize>,
        direct: bool,
    ) -> io::Result<Input> {
        let readers = paths
            .iter()
            .map(|path| Ok(GameReader::try_new(PositionalFile::open(path)?)?))
            .collect::<io::Result<Vec<_>>>()?;
        let direct_files = direct
            .then(|| {
                paths
                    .iter()
                    .map(|path| {
                        File::options()
                            .read(true)
                            .custom_flags(libc::O_DIRECT)
                            .open(path)
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?;
        let mut row_groups: Vec<(usize, usize)> = readers
            .iter()
            .enumerate()
            .flat_map(|(r, reader)| (0..reader.num_row_groups()).map(move |i| (r, i)))
            .collect();
        let rows = |&(r, i): &(usize, usize)| readers[r].metadata().row_group(i).num_rows() as u64;
        let all_rows = row_groups.iter().map(rows).sum();
        let skipped_rows = row_groups[..skip.min(row_groups.len())]
            .iter()
            .map(rows)
            .sum();
        row_groups.drain(..skip.min(row_groups.len()));
        if let Some(limit) = limit {
            row_groups.truncate(limit);
        }
        let mut row_offsets = Vec::with_capacity(row_groups.len() + 1);
        let mut offset = 0;
        row_offsets.push(offset);
        for row_group in &row_groups {
            offset += rows(row_group);
            row_offsets.push(offset);
        }
        Ok(Input {
            readers,
            direct_files,
            row_groups,
            row_offsets,
            skipped_rows,
            all_rows,
        })
    }

    pub fn num_row_groups(&self) -> usize {
        self.row_groups.len()
    }

    pub fn num_rows(&self, row_group: usize) -> u64 {
        self.row_offsets[row_group + 1] - self.row_offsets[row_group]
    }

    /// Total number of rows of the selected row groups.
    pub fn total_rows(&self) -> u64 {
        *self.row_offsets.last().expect("row offsets")
    }

    /// Index of the first row of the (selected) row group, counting from the
    /// first row of the first input, including skipped row groups. This is
    /// the `GameIndex` of the first game of the row group.
    pub fn first_row(&self, row_group: usize) -> u64 {
        self.skipped_rows + self.row_offsets[row_group]
    }

    /// Rows of all inputs, including row groups that are not selected: an
    /// upper bound for game indexes.
    pub fn all_rows(&self) -> u64 {
        self.all_rows
    }

    pub fn read_games(&self, row_group: usize) -> parquet::errors::Result<GameBatch> {
        let (r, i) = self.row_groups[row_group];
        self.readers[r].read_games(i)
    }

    pub fn fetch(&self, row_group: usize) -> parquet::errors::Result<RowGroupBytes> {
        let (r, i) = self.row_groups[row_group];
        let Some(direct_files) = &self.direct_files else {
            return self.readers[r].fetch_row_group(i);
        };
        // One aligned read covering the row group.
        let reader = &self.readers[r];
        let (start, end) = reader.row_group_byte_range(i);
        let file_len = parquet::file::reader::Length::len(reader.input());
        let aligned_start = start / DIRECT_ALIGN as u64 * DIRECT_ALIGN as u64;
        let aligned_end = end.next_multiple_of(DIRECT_ALIGN as u64).min(file_len);
        let mut buf = AlignedBuf::new();
        let dst = buf.resize((aligned_end - aligned_start) as usize);
        // Reads past the end of the file are short, so read until the row
        // group is covered rather than until the aligned length is.
        let needed = (end - aligned_start) as usize;
        let mut filled = 0;
        while filled < needed {
            match direct_files[r].read_at(&mut dst[filled..], aligned_start + filled as u64)? {
                0 => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
                n => filled += n,
            }
        }
        let (vec, offset) = buf.into_parts();
        let bytes = bytes::Bytes::from(vec).slice(
            offset + (start - aligned_start) as usize..offset + (end - aligned_start) as usize,
        );
        Ok(RowGroupBytes::new(start, file_len, bytes))
    }

    pub fn read_games_from(
        &self,
        row_group: usize,
        bytes: RowGroupBytes,
    ) -> parquet::errors::Result<GameBatch> {
        let (r, i) = self.row_groups[row_group];
        self.readers[r].read_games_from(i, bytes)
    }
}

/// Accumulated time of the prefetch threads.
#[derive(Default)]
pub struct PrefetchTimes {
    pub fetch: AtomicU64,
    pub send: AtomicU64,
}

/// Hands out row groups to decoding threads, prefetching their bytes with
/// `io_depth` concurrent reads. Cloned into each decoding thread.
#[derive(Clone)]
pub struct Prefetcher<'a> {
    input: &'a Input,
    /// Fetched row groups, in flight to the decoding threads.
    rx: channel::Receiver<(usize, RowGroupBytes)>,
}

impl<'a> Prefetcher<'a> {
    /// Spawns `io_depth` prefetch threads in the given scope. They finish
    /// once the input is exhausted or all decoding threads are done.
    pub fn spawn<'s>(
        input: &'a Input,
        io_depth: usize,
        times: &'a PrefetchTimes,
        s: &'s thread::Scope<'s, '_>,
    ) -> Prefetcher<'a>
    where
        'a: 's,
    {
        let next_row_group = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = channel::bounded(io_depth);
        for _ in 0..io_depth {
            let tx = tx.clone();
            let next_row_group = Arc::clone(&next_row_group);
            s.spawn(move || {
                loop {
                    let i = next_row_group.fetch_add(1, Ordering::Relaxed);
                    if i >= input.num_row_groups() {
                        break;
                    }
                    let bytes = timed(&times.fetch, || input.fetch(i)).expect("fetch row group");
                    if timed(&times.send, || tx.send((i, bytes))).is_err() {
                        break;
                    }
                }
            });
        }
        Prefetcher { input, rx }
    }

    /// The next row group to decode, or `None` at the end of the input.
    pub fn recv(&self) -> Option<(usize, RowGroupBytes)> {
        self.rx.recv().ok()
    }

    pub fn decode(&self, (i, bytes): (usize, RowGroupBytes)) -> parquet::errors::Result<GameBatch> {
        self.input.read_games_from(i, bytes)
    }
}
