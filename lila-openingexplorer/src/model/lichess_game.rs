use std::convert::{TryFrom, TryInto};

use bytes::{Buf, BufMut};
use serde::{Deserialize, Serialize};
use shakmaty::{ByColor, Color, Outcome};

use crate::model::{read_uint, write_uint, Mode, Month, Speed};

#[derive(Debug)]
pub struct LichessGame {
    pub outcome: Outcome,
    pub speed: Speed,
    pub mode: Mode,
    pub players: ByColor<GamePlayer>,
    pub month: Month,
    pub indexed_player: ByColor<bool>,
    pub indexed_lichess: bool,
}

impl LichessGame {
    pub const SIZE_HINT: usize = 1 + 2 * (1 + 20 + 2) + 2;

    pub fn write<B: BufMut>(&self, buf: &mut B) {
        buf.put_u8(
            match self.speed {
                Speed::UltraBullet => 0,
                Speed::Bullet => 1,
                Speed::Blitz => 2,
                Speed::Rapid => 3,
                Speed::Classical => 4,
                Speed::Correspondence => 5,
            } | (match self.outcome {
                Outcome::Decisive {
                    winner: Color::Black,
                } => 0,
                Outcome::Decisive {
                    winner: Color::White,
                } => 1,
                Outcome::Draw => 2,
            } << 3)
                | (u8::from(self.mode.is_rated()) << 5)
                | (u8::from(self.indexed_player.white) << 6)
                | (u8::from(self.indexed_player.black) << 7),
        );
        self.players.white.write(buf);
        self.players.black.write(buf);
        buf.put_u16_le(u16::from(self.month));
        buf.put_u8(u8::from(self.indexed_lichess));
    }

    pub fn read<B: Buf>(buf: &mut B) -> LichessGame {
        let byte = buf.get_u8();
        let speed = match byte & 7 {
            0 => Speed::UltraBullet,
            1 => Speed::Bullet,
            2 => Speed::Blitz,
            3 => Speed::Rapid,
            4 => Speed::Classical,
            5 => Speed::Correspondence,
            _ => panic!("invalid speed"),
        };
        let outcome = match (byte >> 3) & 3 {
            0 => Outcome::Decisive {
                winner: Color::Black,
            },
            1 => Outcome::Decisive {
                winner: Color::White,
            },
            2 => Outcome::Draw,
            _ => panic!("invalid outcome"),
        };
        let mode = Mode::from_rated((byte >> 5) & 1 == 1);
        let indexed_player = ByColor {
            white: (byte >> 6) & 1 == 1,
            black: (byte >> 7) & 1 == 1,
        };
        let players = ByColor {
            white: GamePlayer::read(buf),
            black: GamePlayer::read(buf),
        };
        let month = buf.get_u16_le().try_into().expect("month");
        let indexed_lichess = buf.get_u8() != 0;
        LichessGame {
            outcome,
            speed,
            mode,
            players,
            month,
            indexed_player,
            indexed_lichess,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct GamePlayer {
    pub name: String,
    pub rating: u16,
}

impl GamePlayer {
    fn write<B: BufMut>(&self, buf: &mut B) {
        write_uint(buf, self.name.len() as u64);
        buf.put_slice(self.name.as_bytes());
        buf.put_u16_le(self.rating);
    }

    fn read<B: Buf>(buf: &mut B) -> GamePlayer {
        let len = usize::try_from(read_uint(buf)).expect("player name len");
        let mut name = vec![0; len];
        buf.copy_to_slice(&mut name);
        GamePlayer {
            name: String::from_utf8(name).expect("name utf-8"),
            rating: buf.get_u16_le(),
        }
    }
}
