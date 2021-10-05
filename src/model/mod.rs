mod game_id;
mod mode;
mod personal;
mod speed;
mod uci;
mod uint;

use std::io::{self, Read, Write};

pub trait Record {
    fn read<R: Read>(reader: &mut R) -> io::Result<Self>
    where
        Self: Sized;
    fn write<W: Write>(&self, writer: &mut W) -> io::Result<()>;
}

pub use game_id::{GameId, InvalidGameId};
pub use mode::{ByMode, Mode};
pub use speed::{BySpeed, Speed};
pub use uci::ByUci;
pub use uint::{read_uint, write_uint};
