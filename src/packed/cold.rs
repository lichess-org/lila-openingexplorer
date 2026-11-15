use crate::packed::{
    PositionKey,
    bits::{BitReader, BitWriter},
    pack_uint, unpack_uint,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ColdKey {
    prefix: [u8; ColdKey::PREFIX_SIZE],
    suffix: u16,
}

impl ColdKey {
    /// Hash bytes used as RocksDB key.
    pub const PREFIX_SIZE: usize = 4;
    /// Further hash bytes that identify the position within the group.
    pub const SUFFIX_SIZE: usize = 2;

    pub fn new(hash: &PositionKey) -> ColdKey {
        ColdKey::from_hash_bytes(hash.as_bytes())
    }

    pub fn from_hash_bytes(hash: &[u8]) -> ColdKey {
        ColdKey {
            prefix: hash[..ColdKey::PREFIX_SIZE].try_into().expect("prefix"),
            suffix: u16::from_be_bytes(
                hash[ColdKey::PREFIX_SIZE..ColdKey::PREFIX_SIZE + ColdKey::SUFFIX_SIZE]
                    .try_into()
                    .expect("suffix"),
            ),
        }
    }

    pub fn prefix(&self) -> &[u8; ColdKey::PREFIX_SIZE] {
        &self.prefix
    }

    pub fn suffix(&self) -> u16 {
        self.suffix
    }
}

/// The games of positions that share a key, as items `(suffix, index)`: the
/// suffix identifies the position within the group (like the bytes of a
/// `ColdKey` after its prefix), the index the game (like a `GameIndex`).
///
/// ```text
/// value := uint(index bits) uint(count) index* gap*
///          (index and gap bit-packed, least significant bit first)
/// index := index bits wide: the bit length of the largest index
/// gap   := suffix - previous suffix (the first from 0), Rice coded with
///          parameter `rice_parameter(count)`: (gap >> k) as unary ones, a
///          zero, the low k bits
/// ```
///
/// Items are sorted by suffix (then index) and unique. Indexes come first,
/// at fixed width, so that a lookup can stop decoding gaps once it is past
/// its suffix, and then read the indexes of the matching items directly.
#[derive(Debug, Default, Clone)]
pub struct ColdGroup {
    items: Vec<(u16, u64)>,
}

impl ColdGroup {
    /// Most bits of an index (of the bit packing).
    pub const MAX_INDEX_BITS: u32 = 56;

    pub fn new() -> ColdGroup {
        ColdGroup::default()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Adds an item, in any order. Duplicates collapse when packing.
    pub fn add(&mut self, suffix: u16, index: u64) {
        self.items.push((suffix, index));
    }

    /// Adds the items of a packed group (so that packing yields the union).
    pub fn merge_unpack_entire(&mut self, value: &[u8]) {
        self.items
            .extend(ColdGroupView::unpack_entire(value).items());
    }

    /// Packs the items. Indexes may have up to `MAX_INDEX_BITS` bits.
    pub fn pack_entire(&mut self, buf: &mut Vec<u8>) {
        self.items.sort_unstable();
        self.items.dedup();
        let max = self.items.iter().map(|&(_, index)| index).max();
        let index_bits = max.map_or(0, |max| u64::BITS - max.leading_zeros());
        assert!(
            index_bits <= ColdGroup::MAX_INDEX_BITS,
            "cold index {max:?} too large"
        );
        let count = self.items.len() as u64;
        let k = rice_parameter(count);
        pack_uint(buf, u64::from(index_bits));
        pack_uint(buf, count);
        let mut bits = BitWriter::new(buf);
        for &(_, index) in &self.items {
            bits.write(index, index_bits);
        }
        let mut previous = 0;
        for &(suffix, _) in &self.items {
            bits.write_rice(u64::from(suffix - previous), k);
            previous = suffix;
        }
        bits.finish_padded();
    }
}

/// A packed `ColdGroup`, read in place.
#[derive(Debug, Clone, Copy)]
pub struct ColdGroupView<'a> {
    k: u32,
    count: u64,
    index_bits: u32,
    bits: &'a [u8],
}

impl<'a> ColdGroupView<'a> {
    /// Views the entire slice as a packed group.
    pub fn unpack_entire(value: &'a [u8]) -> ColdGroupView<'a> {
        let mut rest = value;
        let index_bits = u32::try_from(unpack_uint(&mut rest)).expect("valid index bits");
        debug_assert!(index_bits <= ColdGroup::MAX_INDEX_BITS);
        let count = unpack_uint(&mut rest);
        ColdGroupView {
            k: rice_parameter(count),
            count,
            index_bits,
            bits: rest,
        }
    }

    pub fn len(&self) -> u64 {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn index(&self, i: u64) -> u64 {
        BitReader::at(self.bits, (i * u64::from(self.index_bits)) as usize).read(self.index_bits)
    }

    fn gaps(&self) -> BitReader<'a> {
        BitReader::at(
            self.bits,
            (self.count * u64::from(self.index_bits)) as usize,
        )
    }

    /// The indexes of the items with the suffix (ascending), stopping to
    /// decode gaps once past it.
    pub fn candidates(self, suffix: u16) -> impl Iterator<Item = u64> + 'a {
        let mut gaps = self.gaps();
        let mut current = 0u64;
        let mut i = 0;
        std::iter::from_fn(move || {
            while i < self.count {
                current += gaps.read_rice(self.k);
                let item = i;
                i += 1;
                match current.cmp(&u64::from(suffix)) {
                    std::cmp::Ordering::Less => (),
                    std::cmp::Ordering::Equal => return Some(self.index(item)),
                    std::cmp::Ordering::Greater => {
                        i = self.count;
                        break;
                    }
                }
            }
            None
        })
    }

    /// All items `(suffix, index)`, in order.
    pub fn items(self) -> impl Iterator<Item = (u16, u64)> + 'a {
        let mut gaps = self.gaps();
        let mut suffix = 0u64;
        (0..self.count).map(move |i| {
            suffix += gaps.read_rice(self.k);
            (u16::try_from(suffix).expect("valid suffix"), self.index(i))
        })
    }
}

/// Rice parameter for `count` uniformly distributed suffixes. The gaps are
/// about exponentially distributed with mean `m = 65536 / (count + 1)`, for
/// which `k + 1` beats `k` while `2^k < asinh(1/2) * m`. So this is the
/// smallest `k` with `2^k >= asinh(1/2) * m`, in integer arithmetic. It
/// matches the empirically best parameter for all realistic counts.
fn rice_parameter(count: u64) -> u32 {
    // asinh(1/2) * 65536 = 31536.7...
    (0..16).find(|k| (count + 1) << k >= 31537).unwrap_or(16)
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    fn pack(group: &mut ColdGroup) -> Vec<u8> {
        let mut buf = Vec::new();
        group.pack_entire(&mut buf);
        buf
    }

    fn candidates(value: &[u8], suffix: u16) -> Vec<u64> {
        ColdGroupView::unpack_entire(value)
            .candidates(suffix)
            .collect()
    }

    #[test]
    fn test_group_lookup() {
        let mut group = ColdGroup::new();
        group.add(1, 4);
        group.add(1, 2); // same suffix
        group.add(7, 3);
        group.add(0x0909, 5);
        group.add(7, 3); // duplicate
        let value = pack(&mut group);
        assert_eq!(ColdGroupView::unpack_entire(&value).len(), 4);
        assert_eq!(candidates(&value, 1), vec![2, 4]);
        assert_eq!(candidates(&value, 7), vec![3]);
        assert_eq!(candidates(&value, 0x0909), vec![5]);
        assert!(candidates(&value, 0).is_empty());
        assert!(candidates(&value, 8).is_empty());
        assert!(candidates(&value, u16::MAX).is_empty());

        let value = pack(&mut ColdGroup::new());
        assert!(ColdGroupView::unpack_entire(&value).is_empty());
        assert!(candidates(&value, 0).is_empty());
    }

    /// Items of any width decode to what was added, sorted and unique, with
    /// the least index bits, and lookups find exactly the items with the
    /// suffix.
    #[test]
    fn test_group_roundtrip() {
        arbtest(|u| {
            let index_bits = u.int_in_range(0..=ColdGroup::MAX_INDEX_BITS)?;
            let mut group = ColdGroup::new();
            let mut items: Vec<(u16, u64)> = Vec::new();
            for _ in 0..u.int_in_range(0..=300)? {
                // Few distinct suffixes sometimes, to have duplicates.
                let suffix = if u.ratio(1, 4)? {
                    u.int_in_range(0..=3)?
                } else {
                    u.arbitrary()?
                };
                let index = if index_bits == 0 {
                    0
                } else {
                    u.int_in_range(0..=(1u64 << index_bits) - 1)?
                };
                group.add(suffix, index);
                items.push((suffix, index));
            }
            items.sort_unstable();
            items.dedup();

            let value = pack(&mut group);
            let view = ColdGroupView::unpack_entire(&value);
            let max = items.iter().map(|&(_, index)| index).max();
            assert_eq!(
                view.index_bits,
                max.map_or(0, |max| u64::BITS - max.leading_zeros())
            );
            assert_eq!(view.len(), items.len() as u64);
            assert_eq!(view.items().collect::<Vec<_>>(), items);
            for probe in
                items
                    .iter()
                    .map(|&(suffix, _)| suffix)
                    .chain([u.arbitrary()?, 0, u16::MAX])
            {
                let expected: Vec<u64> = items
                    .iter()
                    .filter(|&&(suffix, _)| suffix == probe)
                    .map(|&(_, index)| index)
                    .collect();
                assert_eq!(candidates(&value, probe), expected);
            }
            Ok(())
        })
        .budget_ms(2000);
    }

    /// Merging packed groups yields the union of their items.
    #[test]
    fn test_merge_is_union() {
        arbtest(|u| {
            let mut all = ColdGroup::new();
            let mut merged = ColdGroup::new();
            for _ in 0..u.int_in_range(0..=4)? {
                let mut part = ColdGroup::new();
                for _ in 0..u.int_in_range(0..=50)? {
                    let suffix = u.int_in_range(0..=20)?;
                    let index = u.int_in_range(0..=20)?;
                    part.add(suffix, index);
                    all.add(suffix, index);
                }
                merged.merge_unpack_entire(&pack(&mut part));
            }
            assert_eq!(pack(&mut merged), pack(&mut all));
            Ok(())
        });
    }

    /// The integer Rice parameter matches its definition.
    #[test]
    fn test_rice_parameter() {
        for count in 1..=100_000u64 {
            let mean_gap = (1 << 16) as f64 / (count + 1) as f64;
            let expected = (0.5f64.asinh() * mean_gap).log2().ceil().max(0.0) as u32;
            assert_eq!(rice_parameter(count), expected, "count {count}");
        }
        // Near the typical group size.
        assert_eq!(rice_parameter(45), 10);
        assert_eq!(rice_parameter(100), 9);
    }

    /// About 12 bits per suffix for groups like those of the full
    /// database (about 45 items).
    #[test]
    fn test_group_size() {
        let games = 8_337_694_503;
        let mut group = ColdGroup::new();
        let mut x = 1u64;
        for i in 0..45 {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            group.add((x >> 48) as u16, games - 1 - i * 1000);
        }
        let len = pack(&mut group).len();
        // 2 header bytes, 45 * 33 index bits, about 45 * 12 gap bits.
        assert!(
            len <= 2 + (45usize * 33 + 45 * 25 / 2).div_ceil(8),
            "{len} bytes"
        );
    }

    #[test]
    #[should_panic(expected = "too large")]
    fn test_index_too_large() {
        let mut group = ColdGroup::new();
        group.add(0, 1 << ColdGroup::MAX_INDEX_BITS);
        pack(&mut group);
    }
}
