use shakmaty::uci::Uci;
use shakmaty::fen::Fen;

enum Speed {
    Ultrabullet,
    Bullet,
    Blitz,
    Rapid,
    Classical,
    Correspondence,
}

enum Mode {
    Casual,
    Rated,
}

struct Query {
    fen: Fen,
    play: Vec<Uci>,
    modes: Option<Vec<Mode>>,
    speeds: Option<Vec<Speed>>,
    player: String,
    color: Color,
    since: Day,
}

struct Day(u16); // days since 2000 or so
