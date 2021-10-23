use std::{
    cmp::Reverse,
    io,
    io::{Read, Write},
    ops::AddAssign,
};

use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr, SpaceSeparator, StringWithSeparator};
use shakmaty::{uci::Uci, ByColor, Color, Outcome};
use smallvec::{smallvec, SmallVec};

use crate::{
    model::{read_uci, write_uci, GameId, LaxDate, Stats},
    util::ByColorDef,
};

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct MasterGameWithId {
    #[serde_as(as = "DisplayFromStr")]
    pub id: GameId,
    #[serde(flatten)]
    pub game: MasterGame,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug)]
pub struct MasterGame {
    pub event: String,
    pub site: String,
    #[serde_as(as = "DisplayFromStr")]
    pub date: LaxDate,
    pub round: String,
    #[serde(flatten, with = "ByColorDef")]
    pub players: ByColor<MasterGamePlayer>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub winner: Option<Color>,
    #[serde_as(as = "StringWithSeparator<SpaceSeparator, Uci>")]
    pub moves: Vec<Uci>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MasterGamePlayer {
    pub name: String,
    pub rating: u16,
}

#[derive(Debug, Default)]
struct Group {
    stats: Stats,
    games: SmallVec<[(u16, GameId); 1]>,
}

impl AddAssign for Group {
    fn add_assign(&mut self, rhs: Group) {
        self.stats += rhs.stats;
        self.games.extend(rhs.games);
    }
}

#[derive(Default, Debug)]
pub struct MasterEntry {
    groups: FxHashMap<Uci, Group>,
}

impl MasterEntry {
    pub const SIZE_HINT: usize = 14;

    pub fn new_single(
        uci: Uci,
        id: GameId,
        outcome: Outcome,
        mover_rating: u16,
        opponent_rating: u16,
    ) -> MasterEntry {
        let mut groups = FxHashMap::with_capacity_and_hasher(1, Default::default());
        groups.insert(
            uci,
            Group {
                stats: Stats::new_single(outcome, mover_rating),
                games: smallvec![(mover_rating.saturating_add(opponent_rating), id)],
            },
        );
        MasterEntry { groups }
    }

    pub fn extend_from_reader<R: Read>(&mut self, reader: &mut R) -> io::Result<()> {
        loop {
            let uci = match read_uci(reader) {
                Ok(uci) => uci,
                Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(err) => return Err(err),
            };

            let group = self.groups.entry(uci).or_default();

            group.stats += Stats::read(reader)?;

            let num_games = reader.read_u8()?;
            group.games.reserve_exact(usize::from(num_games));
            for _ in 0..num_games {
                group
                    .games
                    .push((reader.read_u16::<LittleEndian>()?, GameId::read(reader)?));
            }
        }
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let mut top_games = Vec::new();
        for group in self.groups.values() {
            top_games.extend(&group.games);
        }
        top_games.sort_by_key(|(sort_key, _)| Reverse(*sort_key));
        top_games.truncate(15);

        for (uci, group) in &self.groups {
            write_uci(writer, uci)?;

            group.stats.write(writer)?;

            let num_games = if group.games.len() == 1 {
                1
            } else {
                group.games.iter().filter(|g| top_games.contains(g)).count()
            };
            writer.write_u8(num_games as u8)?;
            for (sort_key, id) in &group.games {
                writer.write_u16::<LittleEndian>(*sort_key)?;
                id.write(writer)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use shakmaty::Square;

    use super::*;

    #[test]
    fn test_master_entry() {
        let uci = Uci::Normal {
            from: Square::E2,
            to: Square::E4,
            promotion: None,
        };
        let game = "aaaaaaaa".parse().unwrap();
        let a = MasterEntry::new_single(uci.clone(), game, Outcome::Draw, 1600, 1700);

        let mut writer = Cursor::new(Vec::with_capacity(MasterEntry::SIZE_HINT));
        a.write(&mut writer).unwrap();
        assert_eq!(
            writer.position() as usize,
            MasterEntry::SIZE_HINT,
            "optimized for single entries"
        );

        let mut reader = Cursor::new(writer.into_inner());
        let mut deserialized = MasterEntry::default();
        deserialized.extend_from_reader(&mut reader).unwrap();

        let group = deserialized.groups.get(&uci).unwrap();
        assert_eq!(group.stats.draws, 1);
        assert_eq!(group.games[0], (1600 + 1700, game));
    }
}
