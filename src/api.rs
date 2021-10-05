use shakmaty::Color;
use shakmaty::uci::Uci;
use shakmaty::fen::Fen;
use shakmaty::variant::Variant;

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
    variant: Variant,
    fen: Fen,
    play: Vec<Uci>,
    modes: Option<Vec<Mode>>,
    speeds: Option<Vec<Speed>>,
    player: String,
    color: Color,
    since: SinceYear,
}

struct SinceYear(u8); // since 2000 or so
