//! Status of a player: indexing progress and the current generation of the
//! warm and cold index.

use arbitrary::{Arbitrary, Unstructured};
use bytes::{Buf, BufMut};
use shakmaty::ByColor;

use crate::{
    packed::{pack_string, pack_uint, unpack_string, unpack_uint},
    player::{
        UserId,
        keys::{Generation, MAX_COLD_BITS, PlayerGameIndex},
        time::UnixMillis,
    },
};

/// ```text
/// status := uint(generation) u8(cold bits white) u8(cold bits black)
///           uint(games) uint(games at last rebuild) uint(latest createdAt)
///           uint(revisit createdAt + 1, or 0) uint(indexed at)
///           uint(revisited at) string(user id) string(name)
/// ```
///
/// Times in milliseconds since the epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStatus {
    pub user: UserId,
    /// The name as last seen in a game (with capitalization).
    pub name: String,
    /// Current generation of the warm and cold index.
    pub generation: Generation,
    /// Bits of the cold buckets of the current generation, per color of
    /// the player.
    pub cold_bits: ByColor<u8>,
    /// Number of indexed games, which is also the next `PlayerGameIndex`.
    pub games: u32,
    /// Games at the last rebuild.
    pub games_at_rebuild: u32,
    /// Latest `createdAt` of games received from lila (indexed or not).
    pub latest_created_at: UnixMillis,
    /// `createdAt` of the earliest game that was ongoing when last seen.
    pub revisit_ongoing: Option<UnixMillis>,
    /// When the last index run finished.
    pub indexed_at: UnixMillis,
    /// When ongoing games were last revisited.
    pub revisited_at: UnixMillis,
}

impl PlayerStatus {
    pub fn new(user: UserId) -> PlayerStatus {
        PlayerStatus {
            name: user.as_str().to_owned(),
            user,
            generation: Generation(0),
            cold_bits: ByColor::new_with(|_| 0),
            games: 0,
            games_at_rebuild: 0,
            latest_created_at: UnixMillis::EPOCH,
            revisit_ongoing: None,
            indexed_at: UnixMillis::EPOCH,
            revisited_at: UnixMillis::EPOCH,
        }
    }

    pub fn next_index(&self) -> PlayerGameIndex {
        PlayerGameIndex(self.games)
    }

    pub fn pack(&self, buf: &mut Vec<u8>) {
        pack_uint(buf, u64::from(self.generation.0));
        buf.put_u8(self.cold_bits.white);
        buf.put_u8(self.cold_bits.black);
        pack_uint(buf, u64::from(self.games));
        pack_uint(buf, u64::from(self.games_at_rebuild));
        self.latest_created_at.pack(buf);
        pack_uint(
            buf,
            self.revisit_ongoing
                .map_or(0, |t| t.0.checked_add(1).expect("valid revisit time")),
        );
        self.indexed_at.pack(buf);
        self.revisited_at.pack(buf);
        pack_string(buf, self.user.as_str());
        pack_string(buf, &self.name);
    }

    pub fn unpack(mut buf: &[u8]) -> PlayerStatus {
        let buf = &mut buf;
        let generation = Generation(unpack_u32(buf));
        let mut unpack_cold_bits = || {
            Some(buf.get_u8())
                .filter(|&bits| bits <= MAX_COLD_BITS)
                .expect("valid cold bits")
        };
        let cold_bits = ByColor {
            white: unpack_cold_bits(),
            black: unpack_cold_bits(),
        };
        let games = unpack_u32(buf);
        let games_at_rebuild = unpack_u32(buf);
        let latest_created_at = UnixMillis::unpack(buf);
        let revisit_ongoing = unpack_uint(buf).checked_sub(1).map(UnixMillis);
        let indexed_at = UnixMillis::unpack(buf);
        let revisited_at = UnixMillis::unpack(buf);
        let user = UserId::from_name(unpack_string(buf)).expect("valid user id");
        let name = unpack_string(buf).to_owned();
        PlayerStatus {
            user,
            name,
            generation,
            cold_bits,
            games,
            games_at_rebuild,
            latest_created_at,
            revisit_ongoing,
            indexed_at,
            revisited_at,
        }
    }
}

impl Arbitrary<'_> for PlayerStatus {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<PlayerStatus> {
        let user: UserId = u.arbitrary()?;
        let name = user
            .as_str()
            .chars()
            .map(|c| {
                u.arbitrary::<bool>()
                    .map(|upper| if upper { c.to_ascii_uppercase() } else { c })
            })
            .collect::<arbitrary::Result<String>>()?;
        let games = u.arbitrary()?;
        Ok(PlayerStatus {
            user,
            name,
            generation: u.arbitrary()?,
            cold_bits: ByColor {
                white: u.int_in_range(0..=MAX_COLD_BITS)?,
                black: u.int_in_range(0..=MAX_COLD_BITS)?,
            },
            games,
            games_at_rebuild: u.int_in_range(0..=games)?,
            latest_created_at: u.arbitrary()?,
            revisit_ongoing: u
                .arbitrary::<bool>()?
                .then(|| u.int_in_range(0..=u64::MAX - 1).map(UnixMillis))
                .transpose()?,
            indexed_at: u.arbitrary()?,
            revisited_at: u.arbitrary()?,
        })
    }
}

fn unpack_u32(buf: &mut &[u8]) -> u32 {
    u32::try_from(unpack_uint(buf)).expect("u32")
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_roundtrip() {
        arbtest(|u| {
            let status: PlayerStatus = u.arbitrary()?;
            let mut buf = Vec::new();
            status.pack(&mut buf);
            assert_eq!(PlayerStatus::unpack(&buf), status);
            Ok(())
        });
    }
}
