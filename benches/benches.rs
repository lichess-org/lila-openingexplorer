use iai::black_box;
use lila_openingexplorer::model::{LichessEntry, Speed};
use shakmaty::{Color, Outcome, Square, uci::UciMove};

fn bench_lichess_write_single() -> Vec<u8> {
    let entry = LichessEntry::new_single(
        black_box(UciMove::Normal {
            from: Square::E2,
            to: Square::E4,
            promotion: None,
        }),
        black_box(Speed::Classical),
        black_box("abcdefgh".parse().expect("game id")),
        black_box(Outcome::Decisive {
            winner: Color::White,
        }),
        black_box(1610),
        black_box(1620),
    );

    let mut buf = Vec::with_capacity(LichessEntry::SIZE_HINT);
    entry.write(&mut buf);
    buf
}

iai::main!(bench_lichess_write_single);
