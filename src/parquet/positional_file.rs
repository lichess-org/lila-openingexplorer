use std::{
    fs::File,
    io,
    io::{BufReader, Read},
    os::unix::fs::FileExt,
    path::Path,
    sync::Arc,
};

use bytes::Bytes;

#[derive(Clone)]
pub struct PositionalFile {
    file: Arc<File>,
    len: u64,
}

impl PositionalFile {
    pub fn open(path: impl AsRef<Path>) -> io::Result<PositionalFile> {
        let file = File::open(path)?;
        Ok(PositionalFile {
            len: file.metadata()?.len(),
            file: Arc::new(file),
        })
    }
}

impl parquet::file::reader::Length for PositionalFile {
    fn len(&self) -> u64 {
        self.len
    }
}

impl parquet::file::reader::ChunkReader for PositionalFile {
    type T = BufReader<PositionalReader>;

    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        Ok(BufReader::with_capacity(
            64 * 1024,
            PositionalReader {
                file: Arc::clone(&self.file),
                pos: start,
            },
        ))
    }

    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        let mut buf = vec![0; length];
        self.file.read_exact_at(&mut buf, start)?;
        Ok(Bytes::from(buf))
    }
}

pub struct PositionalReader {
    file: Arc<File>,
    pos: u64,
}

impl Read for PositionalReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read_at(buf, self.pos)?;
        self.pos += read as u64;
        Ok(read)
    }
}
