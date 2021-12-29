use std::str::FromStr;

use shakmaty::fen::{Fen, ParseFenError};

#[derive(Debug)]
pub struct LaxFen(Fen);

impl From<Fen> for LaxFen {
    fn from(fen: Fen) -> LaxFen {
        LaxFen(fen)
    }
}

impl From<LaxFen> for Fen {
    fn from(LaxFen(fen): LaxFen) -> Fen {
        fen
    }
}

impl FromStr for LaxFen {
    type Err = ParseFenError;

    fn from_str(s: &str) -> Result<LaxFen, ParseFenError> {
        s.replace('_', " ").parse().map(LaxFen)
    }
}
