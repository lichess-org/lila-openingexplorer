use bytes::BufMut;

/// Writes bits to a buffer.
pub struct BitWriter<B> {
    buf: B,
    /// Bits not yet written to the buffer, fewer than 8 between writes.
    acc: u64,
    len: u32,
}

impl<B: BufMut> BitWriter<B> {
    pub fn new(buf: B) -> BitWriter<B> {
        BitWriter {
            buf,
            acc: 0,
            len: 0,
        }
    }

    /// Writes the low `bits` bits of `value` (at most 56).
    #[inline]
    pub fn write(&mut self, value: u64, bits: u32) {
        debug_assert!(bits <= 56 && value >> bits == 0);
        self.acc |= value << self.len;
        self.len += bits;
        while self.len >= 8 {
            self.buf.put_u8(self.acc as u8);
            self.acc >>= 8;
            self.len -= 8;
        }
    }

    /// Writes `value` Rice coded with parameter `k` (at most 56):
    /// `value >> k` as unary ones, a zero, and the low `k` bits.
    pub fn write_rice(&mut self, value: u64, k: u32) {
        let mut quotient = value >> k;
        while quotient >= 32 {
            self.write((1 << 32) - 1, 32);
            quotient -= 32;
        }
        self.write((1 << quotient) - 1, quotient as u32 + 1);
        self.write(value & ((1 << k) - 1), k);
    }

    /// Ends the bits with a 1 bit, so that `BitReader::terminated()` can find
    /// the end, and pads the last byte with zeros.
    pub fn finish_terminated(mut self) -> B {
        self.write(1, 1);
        self.finish_padded()
    }

    /// Pads the last byte with zeros.
    pub fn finish_padded(mut self) -> B {
        if self.len > 0 {
            self.buf.put_u8(self.acc as u8);
        }
        self.buf
    }
}

/// Reads bits written by `BitWriter`. Reads are not bounds checked: reading
/// past the end yields zeros or panics.
pub struct BitReader<'a> {
    bytes: &'a [u8],
    /// Position in bits.
    pos: usize,
    /// End of the bits, in bits.
    len: usize,
    /// Last (up to) 8 bytes, zero padded, so that every read is a fixed size
    /// load.
    tail: u64,
    tail_start: usize,
}

impl<'a> BitReader<'a> {
    /// Reads bits ended with `finish_terminated()`, or `None` if the
    /// terminating bit is missing.
    pub fn terminated(bytes: &'a [u8]) -> Option<BitReader<'a>> {
        let last = *bytes.last().filter(|b| **b != 0)?;
        let mut reader = BitReader::at(bytes, 0);
        reader.len = bytes.len() * 8 - last.leading_zeros() as usize - 1;
        Some(reader)
    }

    /// Reads bits starting at bit `pos`, up to the end of `bytes`.
    pub fn at(bytes: &'a [u8], pos: usize) -> BitReader<'a> {
        let tail = match bytes.last_chunk() {
            Some(chunk) => u64::from_le_bytes(*chunk),
            None => bytes
                .iter()
                .rev()
                .fold(0, |acc, b| (acc << 8) | u64::from(*b)),
        };
        BitReader {
            bytes,
            pos,
            len: bytes.len() * 8,
            tail,
            tail_start: bytes.len().saturating_sub(8),
        }
    }

    /// Whether there are bits left before the end.
    #[inline]
    pub fn has_remaining(&self) -> bool {
        self.pos < self.len
    }

    /// The 64 bits from the current position (zero padded at the end), of
    /// which the first `64 - pos % 8` are valid. Garbage at the very end of
    /// the bytes, where there are no bits left to read.
    #[inline]
    fn peek(&self) -> u64 {
        let start = self.pos / 8;
        (if start < self.tail_start {
            u64::from_le_bytes(*self.bytes[start..].first_chunk().expect("before tail"))
        } else {
            self.tail.wrapping_shr(8 * (start - self.tail_start) as u32)
        }) >> (self.pos % 8)
    }

    /// Reads `bits` bits (at most 56).
    #[inline]
    pub fn read(&mut self, bits: u32) -> u64 {
        debug_assert!(bits <= 56);
        let value = self.peek() & ((1 << bits) - 1);
        self.pos += bits as usize;
        value
    }

    /// Reads a value written with `write_rice()` with parameter `k`.
    pub fn read_rice(&mut self, k: u32) -> u64 {
        let mut quotient = 0;
        loop {
            let valid = 64 - (self.pos % 8) as u32;
            let ones = self.peek().trailing_ones().min(valid);
            quotient += u64::from(ones);
            self.pos += ones as usize;
            if ones < valid {
                self.pos += 1;
                break;
            }
        }
        quotient << k | self.read(k)
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[derive(Debug, Copy, Clone)]
    enum Item {
        Bits(u64, u32),
        Rice(u64, u32),
    }

    fn item(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Item> {
        Ok(if u.arbitrary()? {
            let bits = u.int_in_range(0..=56)?;
            Item::Bits(u.arbitrary::<u64>()? & ((1 << bits) - 1), bits)
        } else {
            let k = u.int_in_range(0..=16)?;
            // Sometimes long unary parts.
            let max = if u.ratio(1, 8)? {
                1 << (k + 8)
            } else {
                1 << (k + 2)
            };
            Item::Rice(u.int_in_range(0..=max)?, k)
        })
    }

    fn items(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Vec<Item>> {
        (0..u.int_in_range(0..=100)?).map(|_| item(u)).collect()
    }

    fn write(writer: &mut BitWriter<&mut Vec<u8>>, items: &[Item]) {
        for &item in items {
            match item {
                Item::Bits(value, bits) => writer.write(value, bits),
                Item::Rice(value, k) => writer.write_rice(value, k),
            }
        }
    }

    fn read(reader: &mut BitReader<'_>, items: &[Item]) {
        for &item in items {
            match item {
                Item::Bits(value, bits) => {
                    assert!(reader.has_remaining() || bits == 0);
                    assert_eq!(reader.read(bits), value);
                }
                Item::Rice(value, k) => assert_eq!(reader.read_rice(k), value),
            }
        }
    }

    #[test]
    fn test_terminated_roundtrip() {
        arbtest(|u| {
            let items = items(u)?;
            let mut buf = Vec::new();
            let mut writer = BitWriter::new(&mut buf);
            write(&mut writer, &items);
            writer.finish_terminated();
            let mut reader = BitReader::terminated(&buf).unwrap();
            read(&mut reader, &items);
            assert!(!reader.has_remaining());
            Ok(())
        });
    }

    #[test]
    fn test_padded_roundtrip() {
        arbtest(|u| {
            let items = items(u)?;
            // Appending after a header.
            let mut buf = vec![0xff; u.int_in_range(0..=3)?];
            let header = buf.len();
            let mut writer = BitWriter::new(&mut buf);
            write(&mut writer, &items);
            writer.finish_padded();
            read(&mut BitReader::at(&buf[header..], 0), &items);
            Ok(())
        });
    }

    #[test]
    fn test_at() {
        let mut buf = Vec::new();
        let mut writer = BitWriter::new(&mut buf);
        for i in 0..20 {
            writer.write(i, 5);
        }
        writer.finish_padded();
        for i in 0..20 {
            assert_eq!(BitReader::at(&buf, 5 * i).read(5), i as u64);
        }
    }
}
