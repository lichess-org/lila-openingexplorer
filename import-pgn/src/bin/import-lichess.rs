use std::{ffi::OsStr, fs::File, io, mem, ops::ControlFlow, path::PathBuf, thread, time::Duration};

use clap::Parser;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use pgn_reader::{KnownOutcome, RawTag, Reader, SanPlus, Skip, Visitor};
use serde::Serialize;
use serde_with::{formats::SpaceSeparator, serde_as, DisplayFromStr, StringWithSeparator};
use shakmaty::Color;
use time::OffsetDateTime;

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
    type Tags = Game;
    type Movetext = Game;
    type Output = ();

    fn begin_tags(&mut self) -> ControlFlow<Self::Output, Self::Tags> {
        ControlFlow::Continue(Game::default())
    }

    fn tag(
        &mut self,
        game: &mut Game,
        name: &[u8],
        value: RawTag<'_>,
    ) -> ControlFlow<Self::Output> {
        if name == b"White" {
            game.white.name = Some(value.decode_utf8().expect("White").into_owned());
        } else if name == b"Black" {
            game.black.name = Some(value.decode_utf8().expect("Black").into_owned());
        } else if name == b"WhiteElo" {
            if value.as_bytes() != b"?" {
                game.white.rating = Some(btoi::btoi(value.as_bytes()).expect("WhiteElo"));
            }
        } else if name == b"BlackElo" {
            if value.as_bytes() != b"?" {
                game.black.rating = Some(btoi::btoi(value.as_bytes()).expect("BlackElo"));
            }
        } else if name == b"TimeControl" {
            game.speed = Some(Speed::from_bytes(value.as_bytes()).expect("TimeControl"));
        } else if name == b"Variant" {
            game.variant = Some(value.decode_utf8().expect("Variant").into_owned());
        } else if name == b"Date" || name == b"UTCDate" {
            game.date = Some(value.decode_utf8().expect("Date").into_owned());
        } else if name == b"WhiteTitle" || name == b"BlackTitle" {
            if value.as_bytes() == b"BOT" {
                return ControlFlow::Break(());
            }
        } else if name == b"Site" {
            game.id = Some(
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
        } else if name == b"Result" {
            match KnownOutcome::from_ascii(value.as_bytes()) {
                Ok(outcome) => game.winner = outcome.winner(),
                Err(_) => return ControlFlow::Break(()),
            }
        } else if name == b"FEN" {
            if value.as_bytes() == b"rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1" {
                // https://github.com/ornicar/lichess-db/issues/40
                game.fen = None;
            } else {
                game.fen = Some(value.decode_utf8().expect("FEN").into_owned());
            }
        }
        ControlFlow::Continue(())
    }

    fn begin_movetext(&mut self, game: Game) -> ControlFlow<Self::Output, Self::Movetext> {
        if game.white.rating.is_none() || game.black.rating.is_none() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(game)
        }
    }

    fn san(&mut self, game: &mut Game, san: SanPlus) -> ControlFlow<Self::Output> {
        game.moves.push(san);
        ControlFlow::Continue(())
    }

    fn begin_variation(&mut self, _game: &mut Game) -> ControlFlow<Self::Output, Skip> {
        ControlFlow::Continue(Skip(true)) // stay in the mainline
    }

    fn end_game(&mut self, game: Game) -> Self::Output {
        self.batch.push(game);
        if self.batch.len() >= self.batch_size {
            self.send();
        }
    }
}

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://localhost:9002")]
    endpoint: String,
    #[arg(long, default_value = "200")]
    batch_size: usize,
    #[arg(long)]
    avoid_utc_hour: Vec<u8>,
    pgns: Vec<PathBuf>,
}

fn main() -> Result<(), io::Error> {
    let args = Args::parse();

    let (tx, rx) = crossbeam::channel::bounded::<Batch>(50);

    let bg = thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(None)
            .build()
            .expect("client");

        while let Ok(batch) = rx.recv() {
            while args
                .avoid_utc_hour
                .contains(&OffsetDateTime::now_utc().hour())
            {
                println!("paused around this time ...");
                thread::sleep(Duration::from_secs(10 * 60));
            }

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

        let mut reader = Reader::new(uncompressed);
        let mut importer = Importer::new(tx.clone(), arg, args.batch_size, &progress);
        reader.visit_all_games(&mut importer)?;
        importer.send();

        progress.finish();
    }

    drop(tx);
    bg.join().expect("bg join");
    Ok(())
}
