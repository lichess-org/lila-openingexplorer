use shakmaty::Color;

enum Speed {
    Ultrabullet,
    Bullet,
    Blitz,
    Rapid,
    Classical,
    Correspondence,
}

enum PerfType {
    Rated,
    Casual,
}

struct HashKey {
    pos: (),
    player: String,
    color: Color,
}

fn main() {
    println!("Hello, world!");
}
