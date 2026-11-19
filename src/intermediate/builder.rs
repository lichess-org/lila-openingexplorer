use crate::{
    intermediate::IntermediateEntry,
    packed::{CellKey, EntryCell, GAMES_PER_CELL, GameIndex, LichessEntry, SubStats},
};

impl LichessEntry {
    /// Adds the game of a record, in any order.
    pub fn add_record(&mut self, record: &IntermediateEntry) {
        self.add_game(
            record.cell_key(),
            record.rating(),
            record.outcome(),
            record.game_index(),
        );
    }
}

/// Reusable buffers for packing entries from sorted records.
#[derive(Default)]
pub struct EntryPacker {
    /// Cells, with the start and end index of their kept games.
    cells: Vec<(CellKey, SubStats, usize, usize)>,
    games: Vec<GameIndex>,
}

impl EntryPacker {
    /// Packs the entry of the sorted intermediate entries of one key.
    pub fn pack_sorted(&mut self, records: &[IntermediateEntry], buf: &mut Vec<u8>) {
        self.cells.clear();
        self.games.clear();
        for run in records.chunk_by(|a, b| a.cell_key() == b.cell_key()) {
            let mut stats = SubStats::default();
            for record in run {
                stats.add(record.rating(), record.outcome());
            }
            let start = self.games.len();
            self.games.extend(
                run[run.len().saturating_sub(GAMES_PER_CELL)..]
                    .iter()
                    .map(IntermediateEntry::game_index),
            );
            self.cells
                .push((run[0].cell_key(), stats, start, self.games.len()));
        }
        // The records are sorted by ascending month within each group.
        for group in self.cells.chunk_by_mut(|a, b| a.0.same_group(&b.0)) {
            group.reverse();
        }

        buf.clear();
        LichessEntry::pack_cells_entire(
            buf,
            self.cells
                .iter()
                .map(|&(key, stats, start, end)| EntryCell {
                    key,
                    stats,
                    games: &self.games[start..end],
                }),
        );
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;
    use shakmaty::{KnownOutcome, Square, packed::PackedUciMove, uci::UciMove};

    use super::*;
    use crate::packed::{GroupHeader, Month, PositionKey, Rating, RatingGroup, Speed};

    #[test]
    fn test_packers_agree() {
        arbtest(|u| {
            let hash: PositionKey = u.arbitrary()?;
            let moves = [
                UciMove::Normal {
                    from: Square::E2,
                    to: Square::E4,
                    promotion: None,
                },
                UciMove::Normal {
                    from: Square::D2,
                    to: Square::D4,
                    promotion: None,
                },
            ];

            let mut records = Vec::new();
            let mut expected = LichessEntry::new();
            let mut seq = 0;
            for _ in 0..u.int_in_range(1..=200)? {
                seq += u.int_in_range(1..=3)?;
                let month = Month::from_months_since_jan_1900(u.int_in_range(1356..=1360)?);
                let m = PackedUciMove::pack(*u.choose(&moves)?);
                let speed = *u.choose(&[Some(Speed::Blitz), Some(Speed::Rapid), None])?;
                let rating = Rating::try_from(u.int_in_range(900..=1300)?).unwrap();
                let header = GroupHeader {
                    speed,
                    rating_group: *u.choose(&[RatingGroup::Group1000, RatingGroup::Group1200])?,
                };
                let outcome: KnownOutcome = u.arbitrary()?;
                records.push(IntermediateEntry::new(
                    hash,
                    month,
                    m,
                    header,
                    rating,
                    outcome,
                    GameIndex::new(seq).unwrap(),
                ));
                expected.add_game(
                    CellKey::new(m, header, month),
                    rating,
                    outcome,
                    GameIndex::new(seq).unwrap(),
                );
            }
            let mut expected_buf = Vec::new();
            expected.pack_entire(&mut expected_buf);

            // Records, in any order.
            let mut shuffled = records.clone();
            for i in (1..shuffled.len()).rev() {
                shuffled.swap(i, u.int_in_range(0..=i)?);
            }
            let mut entry = LichessEntry::new();
            for record in &shuffled {
                entry.add_record(record);
            }
            let mut buf = Vec::new();
            entry.pack_entire(&mut buf);
            assert_eq!(buf, expected_buf);

            // Packer, from sorted records.
            shuffled.sort_unstable_by_key(IntermediateEntry::sort_key);
            EntryPacker::default().pack_sorted(&shuffled, &mut buf);
            assert_eq!(buf, expected_buf);
            Ok(())
        });
    }
}
