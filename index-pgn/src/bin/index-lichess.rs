extern crate pgn_reader;
extern crate memmap;
extern crate madvise;
extern crate btoi;
extern crate rand;
extern crate reqwest;

use std::env;
use std::mem;
use std::str;
use std::cmp::min;
use std::fs::File;
use std::io::Read;
use std::time::Duration;

use memmap::Mmap;
use madvise::{AccessPattern, AdviseMemory};
use pgn_reader::{Visitor, Skip, Reader, San};
use btoi::ParseIntegerError;
use rand::{thread_rng, Rng};
use rand::distributions::OpenClosed01;

const BATCH_SIZE: usize = 50;

const MAX_PLIES: usize = 50;

#[derive(Debug)]
enum TimeControl {
    UltraBullet,
    Bullet,
    Blitz,
    Rapid,
    Classical,
    Correspondence,
}

#[derive(Debug)]
struct TimeControlError;

impl From<ParseIntegerError> for TimeControlError {
    fn from(_: ParseIntegerError) -> TimeControlError {
        TimeControlError { }
    }
}

impl TimeControl {
    fn from_seconds_and_increment(seconds: u64, increment: u64) -> TimeControl {
        let total = seconds + 40 * increment;

        if total < 30 {
            TimeControl::UltraBullet
        } else if total < 180 {
            TimeControl::Bullet
        } else if total < 480 {
            TimeControl::Blitz
        } else if total < 1500 {
            TimeControl::Rapid
        } else if total < 21_600 {
            TimeControl::Classical
        } else {
            TimeControl::Correspondence
        }
    }

    fn from_bytes(bytes: &[u8]) -> Result<TimeControl, TimeControlError> {
        if bytes == b"-" {
            return Ok(TimeControl::Correspondence);
        }

        let mut parts = bytes.splitn(2, |ch| *ch == b'+');
        let seconds = btoi::btou(parts.next().ok_or(TimeControlError)?)?;
        let increment = btoi::btou(parts.next().ok_or(TimeControlError)?)?;
        Ok(TimeControl::from_seconds_and_increment(seconds, increment))
    }
}

struct Indexer<'pgn> {
    client: reqwest::blocking::Client,

    filename: String,
    date: &'pgn [u8],

    white_elo: i16,
    black_elo: i16,
    time_control: TimeControl,
    skip: bool,

    current_game: Vec<u8>,
    plies: usize,
    standard: bool,

    batch: Vec<u8>,
    batch_size: usize,
}

impl<'pgn> Indexer<'pgn> {
    fn new(filename: &str) -> Indexer {
        Indexer {
            client: reqwest::blocking::Client::builder().timeout(Duration::from_secs(60)).build().expect("client"),

            filename: filename.into(),
            date: b"0000.00.00",

            white_elo: 0,
            black_elo: 0,
            time_control: TimeControl::Correspondence,
            skip: true,

            current_game: Vec::new(),
            plies: 0,
            standard: true,

            batch: Vec::new(),
            batch_size: 0,
        }
    }

    fn send(&mut self) {
        if self.batch_size > 0 {
            self.batch_size = 0;

            let mut res = self.client
                .put("http://127.0.0.1:9000/import/lichess")
                .header("Content-Type", "application/vnd.chess-pgn;charset=utf-8")
                .body(mem::replace(&mut self.batch, Vec::new()))
                .send().expect("send batch");

            let mut answer = String::new();
            res.read_to_string(&mut answer).expect("decode response");
            println!("{} d: {} t: {}", self.filename, str::from_utf8(self.date).expect("date is valid utf8"), answer);
            assert!(res.status().is_success());
        }
    }
}

impl<'pgn> Visitor<'pgn> for Indexer<'pgn> {
    type Result = ();

    fn begin_game(&mut self) {
        self.current_game.clear();
        self.plies = 0;
    }

    fn begin_headers(&mut self) {
        self.white_elo = 0;
        self.black_elo = 0;
        self.time_control = TimeControl::Correspondence;
        self.standard = true;
    }

    fn header(&mut self, key: &'pgn [u8], value: &'pgn [u8]) {
        if key == b"WhiteElo" {
            self.white_elo = if value == b"?" { 0 } else { btoi::btoi(value).expect("WhiteElo") };
        } else if key == b"BlackElo" {
            self.black_elo = if value == b"?" { 0 } else { btoi::btoi(value).expect("BlackElo") };
        } else if key == b"TimeControl" {
            self.time_control = TimeControl::from_bytes(value).expect("TimeControl");
        } else if key == b"Variant" {
            self.standard = value == b"Standard";
            if self.standard {
                return; // we add this unconditionally later
            }
        } else if key == b"Date" || key == b"UTCDate" {
            self.date = value;
        }

        let (key, value) = if key == b"Site" {
            (&b"LichessID"[..], value.rsplitn(2, |ch| *ch == b'/').next().expect("Site"))
        } else {
            (key, value)
        };

        self.current_game.push(b'[');
        self.current_game.extend(key);
        self.current_game.extend(b" \"");
        self.current_game.extend(value);
        self.current_game.extend(b"\"]\n");
    }

    fn end_headers(&mut self) -> Skip {
        let rating = (self.white_elo + self.black_elo) / 2;

        let probability = if self.standard {
            self.current_game.extend(b"[Variant \"Standard\"]\n");

            match self.time_control {
                TimeControl::Correspondence | TimeControl::Classical => 1.0,
                TimeControl::Rapid if rating >= 2000 => 1.0,
                TimeControl::Rapid if rating >= 1800 => 2.0 / 5.0,
                TimeControl::Rapid => 1.0 / 8.0,
                TimeControl::Blitz if rating >= 2000 => 1.0,
                TimeControl::Blitz if rating >= 1800 => 1.0 / 4.0,
                TimeControl::Blitz => 1.0 / 15.0,
                TimeControl::Bullet if rating >= 2300 => 1.0,
                TimeControl::Bullet if rating >= 2200 => 4.0 / 5.0,
                TimeControl::Bullet if rating >= 2000 => 1.0 / 4.0,
                TimeControl::Bullet if rating >= 1800 => 1.0 / 7.0,
                _ => 1.0 / 20.0,
            }
        } else {
            if rating >= 1600 { 1.0 } else { 0.5 } // variant games
        };

        self.current_game.push(b'\n');

        let rnd = thread_rng().sample(OpenClosed01);
        let accept = min(self.white_elo, self.black_elo) >= 1500 && probability >= rnd;

        self.skip = !accept;
        Skip(self.skip)
    }

    fn san(&mut self, san: San) {
        if self.plies < MAX_PLIES {
            if self.plies > 0 {
                self.current_game.push(b' ');
            }

            self.current_game.extend(san.to_string().as_bytes());
            self.plies += 1;
        }
    }

    fn begin_variation(&mut self) -> Skip {
        Skip(true) // stay in the mainline
    }

    fn end_game(&mut self, _game: &'pgn [u8]) {
        if !self.skip && self.plies > 8 {
            if self.batch_size >= BATCH_SIZE {
                self.send();
            }

            if self.batch_size > 0 {
                self.batch.extend(b"\n\n\n");
            }

            self.batch.extend(&self.current_game);
            self.batch_size += 1;
        }
    }
}

fn main() {
    for arg in env::args().skip(1) {
        eprintln!("% indexing lichess games from {} ...", arg);
        let file = File::open(&arg).expect("fopen");
        if file.metadata().expect("meta").len() != 0 {
            let pgn = unsafe { Mmap::map(&file).expect("mmap") };
            pgn.advise_memory_access(AccessPattern::Sequential).expect("madvise");

            let mut indexer = Indexer::new(&arg);
            Reader::new(&mut indexer, &pgn[..]).read_all();
            indexer.send(); // send last
        }
    }
}
