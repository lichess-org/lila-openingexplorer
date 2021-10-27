use std::{cmp::min, env, fs::File, io, mem, time::Duration};

use pgn_reader::{BufferedReader, Color, RawHeader, SanPlus, Skip, Visitor};
use rand::{distributions::OpenClosed01, thread_rng, Rng};
use serde::Serialize;
use serde_with::{serde_as, DisplayFromStr, SpaceSeparator, StringWithSeparator};

const BATCH_SIZE: usize = 50;

#[derive(Debug, Serialize, Copy, Clone)]
#[serde(rename_all = "camelCase")]
enum Speed {
    UltraBullet,
    Bullet,
    Blitz,
    Rapid,
    Classical,
    Correspondence,
}

impl Speed {
    fn from_seconds_and_increment(seconds: u64, increment: u64) -> Speed {
        let total = seconds + 40 * increment;

        if total < 30 {
            Speed::UltraBullet
        } else if total < 180 {
            Speed::Bullet
        } else if total < 480 {
            Speed::Blitz
        } else if total < 1500 {
            Speed::Rapid
        } else if total < 21_600 {
            Speed::Classical
        } else {
            Speed::Correspondence
        }
    }

    fn from_bytes(bytes: &[u8]) -> Result<Speed, ()> {
        if bytes == b"-" {
            return Ok(Speed::Correspondence);
        }

        let mut parts = bytes.splitn(2, |ch| *ch == b'+');
        let seconds = btoi::btou(parts.next().ok_or(())?).map_err(|_| ())?;
        let increment = btoi::btou(parts.next().ok_or(())?).map_err(|_| ())?;
        Ok(Speed::from_seconds_and_increment(seconds, increment))
    }
}

struct Importer {
    client: reqwest::blocking::Client,

    current: Game,
    skip: bool,
    batch: Vec<Game>,
}

#[serde_as]
#[derive(Default, Serialize)]
struct Game {
    variant: Option<String>,
    speed: Option<Speed>,
    fen: Option<String>,
    id: Option<String>,
    date: Option<String>,
    white: Player,
    black: Player,
    #[serde_as(as = "Option<DisplayFromStr>")]
    winner: Option<Color>,
    #[serde_as(as = "StringWithSeparator<SpaceSeparator, SanPlus>")]
    moves: Vec<SanPlus>,
}

#[derive(Default, Serialize)]
struct Player {
    name: Option<String>,
    rating: Option<u16>,
}

impl Importer {
    fn new() -> Importer {
        Importer {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("client"),
            current: Game::default(),
            skip: false,
            batch: Vec::with_capacity(BATCH_SIZE),
        }
    }

    pub fn send(&mut self) {
        let mut res = self
            .client
            .put("http://127.0.0.1:9001/import/lichess")
            .json(&self.batch)
            .send()
            .expect("send batch");

        self.batch.clear();
    }
}

impl Visitor for Importer {
    type Result = ();

    fn begin_game(&mut self) {
        self.skip = false;
        self.current = Game::default();
    }

    fn header(&mut self, key: &[u8], value: RawHeader<'_>) {
        if key == b"WhiteElo" {
            if value.as_bytes() != b"?" {
                self.current.white.rating = Some(btoi::btoi(value.as_bytes()).expect("WhiteElo"));
            }
        } else if key == b"BlackElo" {
            if value.as_bytes() != b"?" {
                self.current.black.rating = Some(btoi::btoi(value.as_bytes()).expect("BlackElo"));
            }
        } else if key == b"TimeControl" {
            self.current.speed = Some(Speed::from_bytes(value.as_bytes()).expect("TimeControl"));
        } else if key == b"Variant" {
            self.current.variant = Some(value.decode_utf8().expect("Variant").into_owned());
        } else if key == b"Date" || key == b"UTCDate" {
            self.current.date = Some(value.decode_utf8().expect("Date").into_owned());
        } else if key == b"WhiteTitle" || key == b"BlackTitle" {
            if value.as_bytes() == b"BOT" {
                self.skip = true;
            }
        } else if key == b"Site" {
            self.current.id = Some(
                String::from_utf8(
                    value
                        .as_bytes()
                        .rsplitn(2, |ch| *ch == b'/')
                        .next()
                        .expect("Site")
                        .to_owned(),
                )
                .expect("Site"),
            );
        } else if key == b"Result" {
            match value.as_bytes() {
                b"1-0" => self.current.winner = Some(Color::White),
                b"0-1" => self.current.winner = Some(Color::Black),
                b"1/2-1/2" => self.current.winner = None,
                _ => self.skip = true,
            }
        }
    }

    fn end_headers(&mut self) -> Skip {
        let rating =
            (self.current.white.rating.unwrap_or(0) + self.current.black.rating.unwrap_or(0)) / 2;

        let standard = self
            .current
            .variant
            .as_ref()
            .map_or(false, |name| name != "Standard");

        let probability = if standard {
            match self.current.speed.unwrap_or(Speed::Correspondence) {
                Speed::Correspondence | Speed::Classical => 1.0,
                Speed::Rapid if rating >= 2000 => 1.0,
                Speed::Rapid if rating >= 1800 => 2.0 / 5.0,
                Speed::Rapid => 1.0 / 8.0,
                Speed::Blitz if rating >= 2000 => 1.0,
                Speed::Blitz if rating >= 1800 => 1.0 / 4.0,
                Speed::Blitz => 1.0 / 15.0,
                Speed::Bullet if rating >= 2300 => 1.0,
                Speed::Bullet if rating >= 2200 => 4.0 / 5.0,
                Speed::Bullet if rating >= 2000 => 1.0 / 4.0,
                Speed::Bullet if rating >= 1800 => 1.0 / 7.0,
                _ => 1.0 / 20.0,
            }
        } else {
            if rating >= 1600 {
                1.0
            } else {
                0.5
            } // variant games
        };

        let rnd = thread_rng().sample(OpenClosed01);
        let accept = min(
            self.current.white.rating.unwrap_or(0),
            self.current.black.rating.unwrap_or(0),
        ) >= 1500
            && probability >= rnd
            && !self.skip;

        self.skip = !accept;
        Skip(self.skip)
    }

    fn san(&mut self, san: SanPlus) {
        self.current.moves.push(san);
    }

    fn begin_variation(&mut self) -> Skip {
        Skip(true) // stay in the mainline
    }

    fn end_game(&mut self) {
        if !self.skip {
            self.batch
                .push(mem::replace(&mut self.current, Default::default()));

            if self.batch.len() >= BATCH_SIZE {
                self.send();
            }
        }
    }
}

fn main() -> Result<(), io::Error> {
    for arg in env::args().skip(1) {
        let file = File::open(&arg)?;

        let uncompressed: Box<dyn io::Read> = if arg.ends_with(".bz2") {
            Box::new(bzip2::read::MultiBzDecoder::new(file))
        } else {
            Box::new(file)
        };

        let mut reader = BufferedReader::new(uncompressed);

        let mut importer = Importer::new();
        reader.read_all(&mut importer)?;
    }

    Ok(())
}