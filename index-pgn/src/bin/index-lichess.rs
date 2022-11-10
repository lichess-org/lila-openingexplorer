use std::{cmp::min, ffi::OsStr, fs::File, io, mem, path::PathBuf, thread, time::Duration};

use clap::Parser;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use pgn_reader::{BufferedReader, Color, Outcome, RawHeader, SanPlus, Skip, Visitor};
use serde::Serialize;
use serde_with::{formats::SpaceSeparator, serde_as, DisplayFromStr, StringWithSeparator};

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

struct Batch {
    filename: PathBuf,
    games: Vec<Game>,
}

impl Batch {
    fn last_month(&self) -> &str {
        self.games
            .last()
            .and_then(|g| g.date.as_deref())
            .unwrap_or("")
    }
}

struct Importer<'a> {
    tx: crossbeam::channel::Sender<Batch>,
    filename: PathBuf,
    batch_size: usize,
    progress: &'a ProgressBar,

    current: Game,
    skip: bool,
    batch: Vec<Game>,
}

#[serde_as]
#[derive(Default, Serialize, Debug)]
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

#[derive(Default, Serialize, Debug)]
struct Player {
    name: Option<String>,
    rating: Option<u16>,
}

impl Importer<'_> {
    fn new(
        tx: crossbeam::channel::Sender<Batch>,
        filename: PathBuf,
        batch_size: usize,
        progress: &ProgressBar,
    ) -> Importer<'_> {
        Importer {
            tx,
            filename,
            batch_size,
            current: Game::default(),
            skip: false,
            batch: Vec::with_capacity(batch_size),
            progress,
        }
    }

    pub fn send(&mut self) {
        let batch = Batch {
            filename: self.filename.clone(),
            games: mem::replace(&mut self.batch, Vec::with_capacity(self.batch_size)),
        };
        self.progress.set_message(batch.last_month().to_string());
        self.tx.send(batch).expect("send");
    }
}

impl Visitor for Importer<'_> {
    type Result = ();

    fn begin_game(&mut self) {
        self.skip = false;
        self.current = Game::default();
    }

    fn header(&mut self, key: &[u8], value: RawHeader<'_>) {
        if key == b"White" {
            self.current.white.name = Some(value.decode_utf8().expect("White").into_owned());
        } else if key == b"Black" {
            self.current.black.name = Some(value.decode_utf8().expect("Black").into_owned());
        } else if key == b"WhiteElo" {
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
            match Outcome::from_ascii(value.as_bytes()) {
                Ok(outcome) => self.current.winner = outcome.winner(),
                Err(_) => self.skip = true,
            }
        } else if key == b"FEN" {
            if value.as_bytes() == b"rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1" {
                // https://github.com/ornicar/lichess-db/issues/40
                self.current.fen = None;
            } else {
                self.current.fen = Some(value.decode_utf8().expect("FEN").into_owned());
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
            .map_or(true, |name| name == "Standard");

        let probability = if standard {
            match self.current.speed.unwrap_or(Speed::Correspondence) {
                Speed::Correspondence | Speed::Classical => 100,

                _ if rating >= 2500 => 100,

                Speed::Rapid if rating >= 2200 => 100,
                Speed::Rapid if rating >= 2000 => 83,
                Speed::Rapid if rating >= 1800 => 46,
                Speed::Rapid if rating >= 1600 => 39,

                Speed::Blitz if rating >= 2200 => 38,
                Speed::Blitz if rating >= 2000 => 18,
                Speed::Blitz if rating >= 1600 => 13,

                Speed::Bullet if rating >= 2200 => 48,
                Speed::Bullet if rating >= 2000 => 27,
                Speed::Bullet if rating >= 1800 => 19,
                Speed::Bullet if rating >= 1600 => 18,

                Speed::UltraBullet => 100,

                _ => 2,
            }
        } else {
            // variant games
            if rating >= 1600 {
                100
            } else {
                50
            }
        };

        let accept = min(
            self.current.white.rating.unwrap_or(0),
            self.current.black.rating.unwrap_or(0),
        ) >= 1501
            && self
                .current
                .id
                .as_ref()
                .map_or(false, |id| probability > (java_hash_code(id) % 100))
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
            self.batch.push(mem::take(&mut self.current));

            if self.batch.len() >= self.batch_size {
                self.send();
            }
        }
    }
}

fn java_hash_code(s: &str) -> i32 {
    let mut hash = 0i32;
    for ch in s.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(ch as i32);
    }
    hash
}

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://localhost:9002")]
    endpoint: String,
    #[arg(long, default_value = "2000")]
    batch_size: usize,
    pgns: Vec<PathBuf>,
}

fn main() -> Result<(), io::Error> {
    let args = Args::parse();

    let (tx, rx) = crossbeam::channel::bounded::<Batch>(50);

    let bg = thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("client");

        while let Ok(batch) = rx.recv() {
            let res = client
                .put(format!("{}/import/lichess", args.endpoint))
                .json(&batch.games)
                .send()
                .expect("send batch");

            if !res.status().is_success() {
                println!(
                    "{:?}: {}: {} - {}",
                    batch.filename,
                    batch.last_month(),
                    res.status(),
                    res.text().expect("decode response")
                );
            }
        }
    });

    for arg in args.pgns {
        let file = File::open(&arg)?;
        let progress = ProgressBar::with_draw_target(
            Some(file.metadata()?.len()),
            ProgressDrawTarget::stdout_with_hz(4),
        )
        .with_style(
            ProgressStyle::with_template(
                "{spinner} {prefix} {msg} {wide_bar} {bytes_per_sec:>14} {eta:>7}",
            )
            .unwrap(),
        )
        .with_prefix(format!("{arg:?}"));
        let file = progress.wrap_read(file);

        let uncompressed: Box<dyn io::Read> = if arg.extension() == Some(OsStr::new("bz2")) {
            Box::new(bzip2::read::MultiBzDecoder::new(file))
        } else if arg.extension() == Some(OsStr::new("zst")) {
            Box::new(zstd::Decoder::new(file)?)
        } else {
            Box::new(file)
        };

        let mut reader = BufferedReader::new(uncompressed);
        let mut importer = Importer::new(tx.clone(), arg, args.batch_size, &progress);
        reader.read_all(&mut importer)?;
        importer.send();

        progress.finish();
    }

    drop(tx);
    bg.join().expect("bg join");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::java_hash_code;

    #[test]
    fn test_java_hash_code() {
        assert_eq!(java_hash_code("DXZdUVdv"), 1714524881);
        assert_eq!(java_hash_code("4mn73Yni"), 1587086275);
        assert_eq!(java_hash_code("VFa7wmDN"), 90055046);
        assert_eq!(java_hash_code("rvSvQdIe"), 950841078);
    }
}
