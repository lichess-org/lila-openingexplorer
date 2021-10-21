use std::{
    io,
    io::{Read, Write},
    ops::AddAssign,
};

use byteorder::{LittleEndian, ReadBytesExt};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr, SpaceSeparator, StringWithSeparator};
use shakmaty::{uci::Uci, ByColor, Color, Outcome};
use smallvec::{smallvec, SmallVec};

use crate::{
    model::{read_uci, GameId, Stats},
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
    pub date: String,
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
        todo!()
    }
}
