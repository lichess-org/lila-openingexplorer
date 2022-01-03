use std::str::FromStr;

use shakmaty::fen::{Fen, ParseFenError};

#[derive(Debug)]
pub struct LaxFen(pub Fen);

impl FromStr for LaxFen {
    type Err = ParseFenError;

    fn from_str(s: &str) -> Result<LaxFen, ParseFenError> {
        s.replace('_', " ").parse().map(LaxFen)
    }
}
