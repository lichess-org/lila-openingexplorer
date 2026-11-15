use std::{fs, io, path::PathBuf};

use rocksdb::{BlockBasedIndexType, BlockBasedOptions, DBCompressionType, Options, SstFileWriter};
use thiserror::Error;

use crate::server::db::{COLD_CF, GAME_IDS_CF, GAMES_CF, POSITIONS_CF};

#[derive(Debug, Error)]
pub enum SstError {
    #[error("rocksdb: {0}")]
    RocksDb(#[from] rocksdb::Error),
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

/// An SST file being written under a temporary name.
pub struct SstFile<'a> {
    writer: SstFileWriter<'a>,
    path: PathBuf,
}

impl<'a> SstFile<'a> {
    pub fn create(opts: &'a Options, path: PathBuf) -> Result<SstFile<'a>, SstError> {
        let writer = SstFileWriter::create(opts);
        writer.open(path.with_added_extension("tmp"))?;
        Ok(SstFile { writer, path })
    }

    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<(), SstError> {
        Ok(self.writer.put(key, value)?)
    }

    pub fn file_size(&self) -> u64 {
        self.writer.file_size()
    }

    /// Finishes the file and renames it into place. Returns its size.
    pub fn finish(mut self) -> Result<u64, SstError> {
        self.writer.finish()?;
        fs::rename(self.path.with_added_extension("tmp"), &self.path)?;
        Ok(self.writer.file_size())
    }
}

/// Keep a compressed block if it saves at least 1/16. Blocks of games and
/// positions typically save about 10%, just below the default minimum of 1/8.
pub const MAX_COMPRESSED_BYTES_PER_KB: i32 = 960;

/// Bloom-equivalent bits per key of filters, for about 1% false positives.
pub const FILTER_BITS_PER_KEY: f64 = 10.0;

/// Options of the SST files of a column family. The options that all
/// column families share are set by `table_options()` and
/// `options_with_table()`. `sst_reencode` can override any of them, to
/// evaluate changes of these values.
#[derive(Debug, Clone)]
pub struct SstConfig {
    pub block_size: usize,
    /// Zstd level, or 0 for no compression.
    pub zstd_level: i32,
    /// Use a filter, because lookups often miss and the keys are sparse.
    pub filter: bool,
    /// Store the keys and the values of each data block in separate
    /// sections. Measured on the full lichess database (2026-11-09):
    /// games -3.1%, positions -0.9%, with no slower point lookups, but cold
    /// positions +0.1%.
    pub separate_key_value_in_data_block: bool,
}

impl SstConfig {
    /// Games by game index. Lookups do not miss.
    pub const GAMES: SstConfig = SstConfig {
        block_size: 16 * 1024,
        zstd_level: 3,
        filter: false,
        separate_key_value_in_data_block: true,
    };

    /// Game indexes by game id (masters).
    pub const GAME_IDS: SstConfig = SstConfig {
        separate_key_value_in_data_block: false,
        ..SstConfig::GAMES
    };

    /// Positions. Higher zstd levels save little more, but cost write time.
    pub const POSITIONS: SstConfig = SstConfig {
        block_size: 4096,
        zstd_level: 1,
        filter: true,
        separate_key_value_in_data_block: true,
    };

    /// Cold positions. Keys are dense prefixes of the hash space: every
    /// lookup hits a group, so a filter would not help. The groups are
    /// practically incompressible, except in small databases, where
    /// parquet_to_positions overrides the zstd level.
    pub const COLD_POSITIONS: SstConfig = SstConfig {
        block_size: 4096,
        zstd_level: 0,
        filter: false,
        separate_key_value_in_data_block: false,
    };

    /// The options of the given column family of the explorer databases.
    pub fn for_cf(name: &str) -> Option<&'static SstConfig> {
        match name {
            GAMES_CF => Some(&SstConfig::GAMES),
            GAME_IDS_CF => Some(&SstConfig::GAME_IDS),
            POSITIONS_CF => Some(&SstConfig::POSITIONS),
            COLD_CF => Some(&SstConfig::COLD_POSITIONS),
            _ => None,
        }
    }

    /// Options for writing the SST files.
    pub fn options(&self) -> Options {
        self.options_with_table(&self.table_options())
    }

    /// Options for writing the SST files with the given table options,
    /// which should be based on `table_options()`.
    pub fn options_with_table(&self, table_opts: &BlockBasedOptions) -> Options {
        let mut opts = Options::default();
        if self.zstd_level == 0 {
            opts.set_compression_type(DBCompressionType::None);
        } else {
            opts.set_compression_type(DBCompressionType::Zstd);
            // Window bits and strategy are for zlib only.
            opts.set_compression_options(-14, self.zstd_level, 0, 0);
            opts.set_compression_options_max_compressed_bytes_per_kb(MAX_COMPRESSED_BYTES_PER_KB);
        }
        opts.set_block_based_table_factory(table_opts);
        opts
    }

    /// Table options for writing the SST files, which serve point lookups.
    pub fn table_options(&self) -> BlockBasedOptions {
        let mut table_opts = BlockBasedOptions::default();
        table_opts.set_block_size(self.block_size);
        // Restart interval 16 (the default) with hash index would save ~5% CPU
        // on warm lookups, for 1-2% larger files. We pick size.
        table_opts.set_block_restart_interval(32);
        // Large files: partition the index (and the filter), so that only
        // the top-level index needs to stay in memory.
        table_opts.set_index_type(BlockBasedIndexType::TwoLevelIndexSearch);
        table_opts.set_metadata_block_size(4096);
        // Delta-encode index keys in runs of 16, instead of storing every
        // key in full.
        table_opts.set_index_block_restart_interval(16);
        table_opts.set_separate_key_value_in_data_block(self.separate_key_value_in_data_block);
        if self.filter {
            table_opts.set_ribbon_filter(FILTER_BITS_PER_KEY);
            table_opts.set_partition_filters(true);
            table_opts.set_optimize_filters_for_memory(true);
        }
        table_opts
    }

    /// Table options for reading the SST files. Block size, index type and
    /// compression are taken from the files themselves, but the filter
    /// policy must match to make use of the filters.
    pub fn read_table_options(&self) -> BlockBasedOptions {
        let mut table_opts = BlockBasedOptions::default();
        if self.filter {
            table_opts.set_ribbon_filter(FILTER_BITS_PER_KEY);
        }
        table_opts
    }
}

#[derive(Debug, Default, Clone)]
pub struct SstStats {
    pub keys: u64,
    pub files: u64,
    pub bytes: u64,
    /// Uncompressed key and value bytes.
    pub raw_bytes: u64,
    pub largest_file: u64,
}

impl SstStats {
    pub fn add_file(&mut self, keys: u64, bytes: u64, raw_bytes: u64) {
        self.keys += keys;
        self.files += 1;
        self.bytes += bytes;
        self.raw_bytes += raw_bytes;
        self.largest_file = self.largest_file.max(bytes);
    }

    pub fn merge(&mut self, other: &SstStats) {
        self.keys += other.keys;
        self.files += other.files;
        self.bytes += other.bytes;
        self.raw_bytes += other.raw_bytes;
        self.largest_file = self.largest_file.max(other.largest_file);
    }

    pub fn ratio(&self) -> f64 {
        if self.raw_bytes == 0 {
            0.0
        } else {
            self.bytes as f64 / self.raw_bytes as f64
        }
    }

    pub fn bytes_per_key(&self) -> f64 {
        if self.keys == 0 {
            0.0
        } else {
            self.bytes as f64 / self.keys as f64
        }
    }
}

pub struct RollingSstWriter<'a> {
    opts: &'a Options,
    prefix: PathBuf,
    target_size: u64,
    seq: usize,
    writer: Option<SstFile<'a>>,
    keys_in_file: u64,
    raw_in_file: u64,
    stats: SstStats,
}

impl<'a> RollingSstWriter<'a> {
    pub fn new(opts: &'a Options, prefix: PathBuf, target_size: u64) -> RollingSstWriter<'a> {
        RollingSstWriter {
            opts,
            prefix,
            target_size,
            seq: 0,
            writer: None,
            keys_in_file: 0,
            raw_in_file: 0,
            stats: SstStats::default(),
        }
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), SstError> {
        let w = match &mut self.writer {
            Some(w) => w,
            None => {
                let mut path = self.prefix.clone().into_os_string();
                path.push(format!("_{:04}.sst", self.seq));
                let w = SstFile::create(self.opts, path.into())?;
                self.seq += 1;
                self.writer.insert(w)
            }
        };
        w.put(key, value)?;
        self.keys_in_file += 1;
        self.raw_in_file += (key.len() + value.len()) as u64;
        if w.file_size() >= self.target_size {
            self.finish_file()?;
        }
        Ok(())
    }

    /// Finishes the current file, if any. The next key starts a new file.
    pub fn finish_file(&mut self) -> Result<(), SstError> {
        if let Some(w) = self.writer.take() {
            let size = w.finish()?;
            self.stats
                .add_file(self.keys_in_file, size, self.raw_in_file);
            self.keys_in_file = 0;
            self.raw_in_file = 0;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<SstStats, SstError> {
        self.finish_file()?;
        Ok(self.stats)
    }
}
