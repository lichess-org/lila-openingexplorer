use shakmaty::fen::Fen;
use shakmaty::uci::Uci;
use shakmaty::variant::Variant;
use shakmaty::Color;

use super::model::{Mode, Speed};

struct _Query {
    variant: Variant,
    fen: Fen,
    play: Vec<Uci>,
    modes: Option<Vec<Mode>>,
    speeds: Option<Vec<Speed>>,
    player: String,
    color: Color,
    since: _SinceYear,
}

struct _SinceYear(u8); // since 2000 or so
