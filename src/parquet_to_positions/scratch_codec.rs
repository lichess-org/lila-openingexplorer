//! Compact encoding of the records of a scratch chunk.
//!
//! Records are stored column by column, so that zstd can compress the
//! columns with little entropy (moves, headers, ratings, outcomes) without
//! the random hashes in between:
//!
//! - the hash, without the part implied by the memory partition: the
//!   prefix is stored relative to the first prefix of the partition, in as
//!   few bytes as the partition spans,
//! - the move, the group header and the rating, as in the record,
//! - the game index as the difference to the previous record, together with
//!   the outcome (records of a chunk are in game order, except where row
//!   groups were fetched out of order),
//! - the months as runs (the input is sorted by month).

use bytes::{Buf, BufMut};

use crate::{
    intermediate::IntermediateEntry,
    packed::{pack_uint, unpack_uint},
};

const RECORD: usize = IntermediateEntry::SIZE;
const HASH: usize = IntermediateEntry::KEY_SIZE;

/// Encodes and decodes the records of one memory partition.
#[derive(Copy, Clone, Debug)]
pub struct ScratchCodec {
    /// The smallest hash prefix of the memory partition.
    first_prefix: u32,
    /// Bytes of the prefix relative to `first_prefix`.
    prefix_bytes: usize,
}

impl ScratchCodec {
    pub fn new(memory_partition: usize, memory_partitions: usize) -> ScratchCodec {
        let n = memory_partitions as u64;
        // Prefixes of partition p are in [ceil(p 2^32 / n), ceil((p + 1) 2^32 / n)).
        let first_prefix = ((memory_partition as u64) << 32).div_ceil(n) as u32;
        let max_offset = (1u64 << 32).div_ceil(n) - 1;
        let prefix_bytes = (u64::BITS - max_offset.leading_zeros()).div_ceil(8) as usize;
        ScratchCodec {
            first_prefix,
            prefix_bytes: prefix_bytes.max(1),
        }
    }

    /// Bytes of the hash column per record.
    pub fn hash_bytes(&self) -> usize {
        self.prefix_bytes + HASH - 4
    }

    /// Raw bytes per record, without the variable length columns.
    pub fn fixed_bytes(&self) -> usize {
        self.hash_bytes() + 2 + 1 + 2
    }

    /// Expected raw bytes per record, if each game has on average `rate`
    /// records in the memory partition. The month runs and the chunk header
    /// are negligible.
    pub fn record_bytes(&self, rate: f64) -> f64 {
        self.fixed_bytes() as f64 + game_bytes(rate)
    }

    pub fn encode(&self, records: &[[u8; RECORD]], out: &mut Vec<u8>) {
        let n = records.len();
        out.clear();
        out.reserve(8 + n * (self.fixed_bytes() + 2) + 16);
        out.put_u32_le(n as u32);
        let gi_len_at = out.len();
        out.put_u32_le(0);
        for r in records {
            let prefix = u32::from_be_bytes(r[..4].try_into().expect("prefix"));
            let offset = prefix.wrapping_sub(self.first_prefix);
            debug_assert!(u64::from(offset) < 1 << (8 * self.prefix_bytes));
            out.put_slice(&offset.to_be_bytes()[4 - self.prefix_bytes..]);
            out.put_slice(&r[4..HASH]);
        }
        for r in records {
            out.put_slice(&r[12..14]);
        }
        for r in records {
            out.put_u8(r[14]);
        }
        for r in records {
            out.put_slice(&r[22..24]);
        }
        let gi_start = out.len();
        let mut prev = 0;
        for r in records {
            let game_and_outcome = (&r[17..22]).get_uint(5);
            let (game, outcome) = (game_and_outcome >> 2, game_and_outcome & 0b11);
            match game.checked_sub(prev) {
                Some(delta) => pack_uint(out, (delta + 1) << 2 | outcome),
                None => {
                    // Escape, then the game index itself.
                    pack_uint(out, outcome);
                    pack_uint(out, game);
                }
            }
            prev = game;
        }
        let gi_len = (out.len() - gi_start) as u32;
        out[gi_len_at..gi_len_at + 4].copy_from_slice(&gi_len.to_le_bytes());
        // Month runs.
        let mut i = 0;
        while i < n {
            let month = &records[i][15..17];
            let mut j = i + 1;
            while j < n && &records[j][15..17] == month {
                j += 1;
            }
            pack_uint(out, (j - i) as u64);
            out.put_slice(month);
            i = j;
        }
    }

    /// Calls `f` with each record of an encoded chunk.
    #[inline]
    pub fn decode(&self, mut raw: &[u8], mut f: impl FnMut(IntermediateEntry)) {
        let n = raw.get_u32_le() as usize;
        let gi_len = raw.get_u32_le() as usize;
        let hb = self.hash_bytes();
        let (hashes, rest) = raw.split_at(n * hb);
        let (moves, rest) = rest.split_at(n * 2);
        let (headers, rest) = rest.split_at(n);
        let (ratings, rest) = rest.split_at(n * 2);
        let (mut games, mut months) = rest.split_at(gi_len);
        let mut game = 0;
        let mut run = 0u64;
        let mut month = [0; 2];
        let pb = self.prefix_bytes;
        for i in 0..n {
            let mut r = [0u8; RECORD];
            let h = &hashes[i * hb..(i + 1) * hb];
            let mut offset = [0u8; 4];
            offset[4 - pb..].copy_from_slice(&h[..pb]);
            let prefix = self.first_prefix.wrapping_add(u32::from_be_bytes(offset));
            r[..4].copy_from_slice(&prefix.to_be_bytes());
            r[4..HASH].copy_from_slice(&h[pb..]);
            r[12..14].copy_from_slice(&moves[i * 2..i * 2 + 2]);
            r[14] = headers[i];
            if run == 0 {
                run = unpack_uint(&mut months);
                month.copy_from_slice(&months[..2]);
                months.advance(2);
            }
            run -= 1;
            r[15..17].copy_from_slice(&month);
            let v = unpack_uint(&mut games);
            game = match v >> 2 {
                0 => unpack_uint(&mut games),
                delta => game + delta - 1,
            };
            let game_and_outcome = game << 2 | (v & 0b11);
            r[17..22].copy_from_slice(&game_and_outcome.to_be_bytes()[3..]);
            r[22..24].copy_from_slice(&ratings[i * 2..i * 2 + 2]);
            f(IntermediateEntry(r));
        }
        debug_assert!(games.is_empty() && months.is_empty() && run == 0);
    }
}

/// Expected bytes of the game index column per record. Assuming that each
/// game independently has a record with probability `rate`, the difference
/// d to the previous record of the same thread is geometric with
/// P(d >= m) = (1 - rate)^m. The varint of (d + 1) << 2 takes more than k
/// bytes if d >= 2^(7k - 2) - 1. Records of the same game in the same
/// partition make d = 0 more likely, so this overestimates.
fn game_bytes(rate: f64) -> f64 {
    let miss = 1.0 - rate.clamp(0.0, 1.0);
    1.0 + (1..=5)
        .map(|k| miss.powf(((1u64 << (7 * k - 2)) - 1) as f64))
        .sum::<f64>()
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;
    use crate::parquet_to_positions::partition::memory_partition;

    #[test]
    fn test_game_bytes() {
        assert_eq!(game_bytes(1.0), 1.0);
        // 3 x 122 partitions, about 41 cold records per game.
        let bytes = game_bytes(41.0 / 366.0);
        assert!(1.02 < bytes && bytes < 1.04, "{bytes}");
        // Many partitions: mostly 2 bytes.
        let bytes = game_bytes(41.0 / 20_000.0);
        assert!(1.9 < bytes && bytes < 2.1, "{bytes}");
    }

    #[test]
    fn test_roundtrip() {
        arbtest(|u| {
            let memory_partitions: usize = u.int_in_range(1..=70_000)?;
            let p = u.int_in_range(0..=memory_partitions - 1)?;
            let lo = ((p as u64) << 32).div_ceil(memory_partitions as u64);
            let hi = (((p + 1) as u64) << 32).div_ceil(memory_partitions as u64);
            let codec = ScratchCodec::new(p, memory_partitions);
            let mut records = Vec::new();
            let mut game: u64 = u.int_in_range(0..=(1 << 38) - 1)?;
            let mut month: [u8; 2] = u.arbitrary()?;
            for _ in 0..u.int_in_range(0..=500)? {
                let mut r: [u8; RECORD] = u.arbitrary()?;
                let prefix = u.int_in_range(lo..=hi - 1)? as u32;
                r[..4].copy_from_slice(&prefix.to_be_bytes());
                assert_eq!(
                    memory_partition(r[..HASH].try_into().unwrap(), memory_partitions),
                    p
                );
                if u.ratio(1, 4)? {
                    game = u.int_in_range(0..=(1 << 38) - 1)?;
                } else if u.arbitrary()? {
                    game = (game + u.int_in_range(0..=40)?).min((1 << 38) - 1);
                }
                if u.ratio(1, 8)? {
                    month = u.arbitrary()?;
                }
                r[15..17].copy_from_slice(&month);
                let outcome = u64::from(r[21] & 0b11);
                r[17..22].copy_from_slice(&(game << 2 | outcome).to_be_bytes()[3..]);
                records.push(r);
            }
            let mut encoded = Vec::new();
            codec.encode(&records, &mut encoded);
            let mut decoded = Vec::new();
            codec.decode(&encoded, |entry| decoded.push(entry.0));
            assert_eq!(decoded, records);
            Ok(())
        });
    }
}
