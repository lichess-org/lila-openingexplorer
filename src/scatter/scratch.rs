use std::{
    fs::{self, File},
    io,
    ops::Range,
    os::unix::fs::{FileExt, OpenOptionsExt},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use crate::util::{AlignedBuf, DIRECT_ALIGN, Gauge};

pub struct ChunkCompressor {
    /// `None` for no compression.
    zstd: Option<zstd::bulk::Compressor<'static>>,
    buf: Vec<u8>,
}

impl ChunkCompressor {
    pub fn new(zstd_level: i32) -> ChunkCompressor {
        ChunkCompressor {
            zstd: (zstd_level != 0)
                .then(|| zstd::bulk::Compressor::new(zstd_level).expect("zstd compressor")),
            buf: Vec::new(),
        }
    }

    pub fn compress<'a>(&'a mut self, raw: &'a [u8]) -> &'a [u8] {
        match &mut self.zstd {
            None => raw,
            Some(zstd) => {
                self.buf.clear();
                self.buf.reserve(zstd::zstd_safe::compress_bound(raw.len()));
                zstd.compress_to_buffer(raw, &mut self.buf)
                    .expect("compress");
                &self.buf
            }
        }
    }
}

struct ChunkDecompressor {
    /// `None` for no compression.
    zstd: Option<zstd::bulk::Decompressor<'static>>,
}

impl ChunkDecompressor {
    fn new(zstd_level: i32) -> ChunkDecompressor {
        ChunkDecompressor {
            zstd: (zstd_level != 0)
                .then(|| zstd::bulk::Decompressor::new().expect("zstd decompressor")),
        }
    }

    fn decompress(&mut self, compressed: &[u8], dst: &mut [u8]) {
        let n = match &mut self.zstd {
            None => {
                dst.copy_from_slice(compressed);
                compressed.len()
            }
            Some(zstd) => zstd
                .decompress_to_buffer(compressed, dst)
                .expect("decompress"),
        };
        assert_eq!(n, dst.len(), "chunk length");
    }
}

#[derive(Clone)]
struct ChunkMeta {
    offset: u64,
    raw_len: u32,
    compressed_len: u32,
}

impl ChunkMeta {
    fn end(&self) -> u64 {
        self.offset + u64::from(self.compressed_len)
    }
}

/// Bytes to read at once when loading a scratch file. Hard disks seek away
/// to other files between the reads.
const READ_RUN: u64 = 16 << 20;

/// A scratch file holding compressed chunks of records.
///
/// Written through the page cache: kernel writeback writes a long run of
/// each file at a time, even while many files grow. Hard disks need that
/// (each direct write would be a seek away from the previous one), and fast
/// disks are no slower for it.
pub struct ScratchFile {
    path: PathBuf,
    write_file: File,
    /// Handle for reading back: if `direct_reads`, with `O_DIRECT`.
    read_file: File,
    direct_reads: bool,
    zstd_level: i32,
    len: u64,
    chunks: Vec<ChunkMeta>,
}

impl ScratchFile {
    pub fn create(path: PathBuf, zstd_level: i32, direct_reads: bool) -> io::Result<ScratchFile> {
        let write_file = File::create(&path)?;
        let mut read_options = File::options();
        read_options.read(true);
        if direct_reads {
            read_options.custom_flags(libc::O_DIRECT);
        }
        let read_file = read_options.open(&path)?;
        Ok(ScratchFile {
            path,
            write_file,
            read_file,
            direct_reads,
            zstd_level,
            len: 0,
            chunks: Vec::new(),
        })
    }

    /// Bytes occupied on disk.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Uncompressed bytes of all chunks.
    pub fn raw_len(&self) -> u64 {
        self.chunks
            .iter()
            .map(|chunk| u64::from(chunk.raw_len))
            .sum()
    }

    pub fn append(&mut self, raw_len: usize, compressed: &[u8], usage: &Gauge) -> io::Result<()> {
        self.write_file.write_all_at(compressed, self.len)?;
        self.chunks.push(ChunkMeta {
            offset: self.len,
            raw_len: raw_len as u32,
            compressed_len: compressed.len() as u32,
        });
        self.len += compressed.len() as u64;
        usage.add(compressed.len() as u64);
        Ok(())
    }

    /// Loads and decompresses all chunks in parallel, reading runs of
    /// consecutive chunks at once. Each thread starts with a state from
    /// `init` and calls `f` with the index and the raw bytes of each chunk
    /// it loads (which `f` may take). Returns the states of all threads.
    pub fn load<S: Send>(
        &self,
        threads: usize,
        init: impl Fn() -> S + Sync,
        f: impl Fn(&mut S, usize, &mut Vec<u8>) + Sync,
    ) -> Vec<S> {
        let runs = self.runs();
        let next = AtomicUsize::new(0);
        let states = Mutex::new(Vec::with_capacity(threads));

        thread::scope(|s| {
            for _ in 0..threads.max(1) {
                s.spawn(|| {
                    let mut state = init();
                    let mut decompressor = ChunkDecompressor::new(self.zstd_level);
                    let mut compressed = AlignedBuf::new();
                    let mut raw = Vec::new();
                    while let Some(run) = runs.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let (start, bytes) = self
                            .read_run(run.clone(), &mut compressed)
                            .expect("read chunks");
                        for i in run.clone() {
                            let chunk = &self.chunks[i];
                            let offset = (chunk.offset - start) as usize;
                            raw.clear();
                            raw.resize(chunk.raw_len as usize, 0);
                            decompressor.decompress(
                                &bytes[offset..offset + chunk.compressed_len as usize],
                                &mut raw,
                            );
                            f(&mut state, i, &mut raw);
                        }
                    }
                    states.lock().expect("lock states").push(state);
                });
            }
        });

        states.into_inner().expect("states")
    }

    /// Ranges of consecutive chunks that span about `READ_RUN` bytes.
    fn runs(&self) -> Vec<Range<usize>> {
        let mut runs = Vec::new();
        let mut first = 0;
        for (i, chunk) in self.chunks.iter().enumerate() {
            if chunk.end() - self.chunks[first].offset >= READ_RUN {
                runs.push(first..i + 1);
                first = i + 1;
            }
        }
        if first < self.chunks.len() {
            runs.push(first..self.chunks.len());
        }
        runs
    }

    /// Reads a non-empty run of chunks with a single read. Returns the
    /// offset of the first byte read and the bytes.
    fn read_run<'a>(
        &self,
        run: Range<usize>,
        buf: &'a mut AlignedBuf,
    ) -> io::Result<(u64, &'a [u8])> {
        let (mut start, end) = (
            self.chunks[run.start].offset,
            self.chunks[run.end - 1].end(),
        );
        let mut aligned_end = end;
        if self.direct_reads {
            start -= start % DIRECT_ALIGN as u64;
            aligned_end = end.next_multiple_of(DIRECT_ALIGN as u64);
        }
        let dst = &mut buf.resize((aligned_end - start) as usize)[..(aligned_end - start) as usize];
        // The aligned end can be past the end of the file.
        let needed = (end - start) as usize;
        let mut filled = 0;
        while filled < needed {
            match self
                .read_file
                .read_at(&mut dst[filled..], start + filled as u64)
            {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => filled += n,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => return Err(err),
            }
        }
        Ok((start, &dst[..needed]))
    }

    pub fn remove(self, usage: &Gauge) -> io::Result<()> {
        drop(self.write_file);
        drop(self.read_file);
        fs::remove_file(&self.path)?;
        usage.sub(self.len);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{env, process};

    use super::*;

    fn chunk(i: usize) -> Vec<u8> {
        let len = 600_000 + i * 12_345;
        (0..len).map(|j| (i * 31 + j * 7 + j / 251) as u8).collect()
    }

    fn roundtrip(zstd_level: i32, direct_reads: bool) {
        let path = env::temp_dir().join(format!(
            "scratch-test-{}-{zstd_level}-{direct_reads}",
            process::id()
        ));
        let mut file = match ScratchFile::create(path, zstd_level, direct_reads) {
            Ok(file) => file,
            // The file system does not support O_DIRECT.
            Err(err) if err.raw_os_error() == Some(libc::EINVAL) => return,
            Err(err) => panic!("create scratch file: {err}"),
        };
        let usage = Gauge::default();
        let mut compressor = ChunkCompressor::new(zstd_level);
        let chunks = 64;
        for i in 0..chunks {
            let raw = chunk(i);
            file.append(raw.len(), compressor.compress(&raw), &usage)
                .expect("append");
        }
        assert_eq!(
            file.raw_len(),
            (0..chunks).map(|i| chunk(i).len() as u64).sum::<u64>()
        );
        if zstd_level == 0 {
            assert!(file.runs().len() > 2, "several runs");
        }

        let mut seen = file
            .load(3, Vec::new, |seen, i, raw| {
                assert_eq!(*raw, chunk(i), "chunk {i}");
                seen.push(i);
            })
            .concat();
        seen.sort_unstable();
        assert_eq!(seen, (0..chunks).collect::<Vec<_>>());

        file.remove(&usage).expect("remove");
        usage.wait_until(|bytes| bytes == 0);
    }

    #[test]
    fn test_scratch_file_roundtrip() {
        for zstd_level in [0, 1] {
            for direct_reads in [false, true] {
                roundtrip(zstd_level, direct_reads);
            }
        }
    }
}
