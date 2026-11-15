use std::path::Path;

use clap::ValueEnum;

use crate::util::system::rotational;

/// When to write files with `O_DIRECT`, bypassing the page cache.
#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum DirectWrites {
    /// Unless the files are on a rotational disk.
    Auto,
    Always,
    Never,
}

impl DirectWrites {
    /// Whether to write files in `dir` with `O_DIRECT`.
    ///
    /// Fast disks are limited by the page cache. Hard disks need it: kernel
    /// writeback writes a long run of each file at a time, even while many
    /// files grow, but each direct write is a seek away from the previous
    /// one.
    pub fn resolve(self, dir: &Path) -> bool {
        match self {
            DirectWrites::Auto => rotational(dir) != Some(true),
            DirectWrites::Always => true,
            DirectWrites::Never => false,
        }
    }
}
