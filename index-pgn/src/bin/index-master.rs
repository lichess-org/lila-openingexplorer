extern crate pgn_reader;
extern crate memmap;
extern crate madvise;
extern crate reqwest;

use std::env;
use std::fs::File;

use memmap::Mmap;
use madvise::{AccessPattern, AdviseMemory};
use pgn_reader::{Visitor, Skip, Reader};

struct Indexer;

impl<'pgn> Visitor<'pgn> for Indexer {
    type Result = ();

    fn end_headers(&mut self) -> Skip {
        Skip(true)
    }

    fn end_game(&mut self, game: &'pgn [u8]) {

        let res = reqwest::blocking::Client::new()
            .put("http://localhost:9000/import/master")
            .header("Content-Type", "application/vnd.chess-pgn;charset=utf-8")
            .body(game.to_owned())
            .send().expect("send game");

        let answer = res.text().expect("decode response");
        println!("-> {}", answer);
    }
}

fn main() {
    for arg in env::args().skip(1) {
        eprintln!("% indexing master games from {} ...", arg);
        let file = File::open(&arg).expect("fopen");
        let pgn = unsafe { Mmap::map(&file).expect("mmap") };
        pgn.advise_memory_access(AccessPattern::Sequential).expect("madvise");

        Reader::new(&mut Indexer, &pgn[..]).read_all();
    }
}
