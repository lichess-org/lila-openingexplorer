/// Alignment of buffers, offsets and lengths for `O_DIRECT` reads and writes.
pub const DIRECT_ALIGN: usize = 4096;

/// A zero filled buffer aligned to `DIRECT_ALIGN`.
pub struct AlignedBuf {
    vec: Vec<u8>,
    offset: usize,
}

impl Default for AlignedBuf {
    fn default() -> AlignedBuf {
        AlignedBuf::new()
    }
}

impl AlignedBuf {
    pub fn new() -> AlignedBuf {
        AlignedBuf {
            vec: Vec::new(),
            offset: 0,
        }
    }

    /// Resizes to `len` bytes (rounded up to the alignment) and returns
    /// them, zero filled.
    pub fn resize(&mut self, len: usize) -> &mut [u8] {
        let len = len.next_multiple_of(DIRECT_ALIGN);
        if self.vec.capacity() < len + DIRECT_ALIGN {
            self.vec = Vec::with_capacity(len + DIRECT_ALIGN);
        }
        self.offset = self.vec.as_ptr().align_offset(DIRECT_ALIGN);
        self.vec.clear();
        self.vec.resize(self.offset + len, 0);
        &mut self.vec[self.offset..]
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.vec[self.offset..]
    }

    /// The underlying vector and the offset of the aligned bytes within it.
    pub fn into_parts(self) -> (Vec<u8>, usize) {
        (self.vec, self.offset)
    }
}
