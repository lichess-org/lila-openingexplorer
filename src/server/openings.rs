use std::{fs, io, path::Path, sync::RwLock, time::Duration};

use nohash_hasher::IntMap;
use serde::Serialize;
use shakmaty::{
    Chess, EnPassantMode, Position,
    san::San,
    variant::{Variant, VariantPosition},
    zobrist::Zobrist64,
};
use thiserror::Error;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Opening {
    pub eco: String,
    pub name: String,
}

#[derive(Debug, Error)]
pub enum OpeningsError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("download: {0}")]
    Download(#[from] reqwest::Error),
    #[error("invalid tsv line {line}: {reason}")]
    Tsv { line: usize, reason: String },
    #[error("duplicate opening position for {0}")]
    Duplicate(String),
}

pub const TSV_FILES: [&str; 5] = ["a.tsv", "b.tsv", "c.tsv", "d.tsv", "e.tsv"];

/// Opening names by position (standard chess only).
#[derive(Default)]
pub struct Openings {
    data: IntMap<Zobrist64, Opening>,
}

impl Openings {
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The opening names to start with: from the directory, if any (logged),
    /// or none.
    pub fn initial(dir: Option<&Path>) -> Result<Openings, OpeningsError> {
        let Some(dir) = dir else {
            return Ok(Openings::default());
        };
        let openings = Openings::load_dir(dir)?;
        log::info!(
            "loaded {} opening names from {}",
            openings.len(),
            dir.display()
        );
        Ok(openings)
    }

    pub fn load_dir(dir: &Path) -> Result<Openings, OpeningsError> {
        let mut openings = Openings::default();
        for file in TSV_FILES {
            openings.load_tsv(&fs::read_to_string(dir.join(file))?)?;
        }
        Ok(openings)
    }

    pub async fn download() -> Result<Openings, OpeningsError> {
        let client = reqwest::Client::builder()
            .user_agent("lila-openingexplorer/4")
            .timeout(Duration::from_secs(60))
            .build()?;
        let mut openings = Openings::default();
        for file in TSV_FILES {
            let tsv = client
                .get(format!(
                    "https://raw.githubusercontent.com/lichess-org/chess-openings/master/{file}"
                ))
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            openings.load_tsv(&tsv)?;
        }
        Ok(openings)
    }

    pub fn load_tsv(&mut self, tsv: &str) -> Result<(), OpeningsError> {
        for (i, line) in tsv.lines().enumerate().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            let mut fields = line.split('\t');
            let (Some(eco), Some(name), Some(pgn)) = (fields.next(), fields.next(), fields.next())
            else {
                return Err(OpeningsError::Tsv {
                    line: i + 1,
                    reason: "expected 3 columns".to_owned(),
                });
            };
            let mut pos = Chess::default();
            for token in pgn.split(' ') {
                if let Ok(san) = token.parse::<San>() {
                    let m = san.to_move(&pos).map_err(|err| OpeningsError::Tsv {
                        line: i + 1,
                        reason: err.to_string(),
                    })?;
                    pos.play_unchecked(m);
                }
            }
            let opening = Opening {
                eco: eco.to_owned(),
                name: name.to_owned(),
            };
            if self
                .data
                .insert(pos.zobrist_hash(EnPassantMode::Legal), opening)
                .is_some()
            {
                return Err(OpeningsError::Duplicate(format!("{eco} {name}")));
            }
        }
        Ok(())
    }

    pub fn classify_exact(&self, pos: &VariantPosition) -> Option<&Opening> {
        if matches!(
            pos.variant(),
            Variant::Chess | Variant::Crazyhouse | Variant::ThreeCheck | Variant::KingOfTheHill
        ) {
            self.data.get(&pos.zobrist_hash(EnPassantMode::Legal))
        } else {
            None
        }
    }
}

/// Keeps the opening names up to date (downloading them about every three
/// hours), calling `on_change` after each update.
pub async fn periodic_refresh(openings: &RwLock<Openings>, on_change: impl Fn()) {
    loop {
        match Openings::download().await {
            Ok(downloaded) => {
                log::info!("refreshed {} opening names", downloaded.len());
                *openings.write().expect("write openings") = downloaded;
                on_change();
                tokio::time::sleep(Duration::from_secs(60 * 167)).await;
            }
            Err(err) => {
                log::error!("failed to refresh opening names: {err}");
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use shakmaty::uci::UciMove;

    use super::*;

    #[test]
    fn test_load_tsv() {
        let mut openings = Openings::default();
        openings
            .load_tsv("eco\tname\tpgn\nB00\tKing's Pawn\t1. e4\nA40\tQueen's Pawn\t1. d4\n")
            .unwrap();
        assert_eq!(openings.len(), 2);
        let mut pos = VariantPosition::new(Variant::Chess);
        assert_eq!(openings.classify_exact(&pos), None);
        let m = "e2e4".parse::<UciMove>().unwrap().to_move(&pos).unwrap();
        pos.play_unchecked(m);
        assert_eq!(openings.classify_exact(&pos).unwrap().eco, "B00");
    }

    /// Plays SAN moves from the start position of the variant.
    fn play(variant: Variant, sans: &str) -> VariantPosition {
        let mut pos = VariantPosition::new(variant);
        for san in sans.split_whitespace() {
            let m = san.parse::<San>().unwrap().to_move(&pos).unwrap();
            pos.play_unchecked(m);
        }
        pos
    }

    #[test]
    fn test_classify_variants() {
        let mut openings = Openings::default();
        openings
            .load_tsv(
                "eco\tname\tpgn\n\
                 C44\tKing's Knight Opening\t1. e4 e5 2. Nf3\n\
                 B01\tScandinavian Defense: Mieses-Kotroc Variation\t1. e4 d5 2. exd5 Qxd5\n\
                 B00\tBarnes Defense\t1. e4 f6 2. Qh5+\n",
            )
            .unwrap();
        let eco = |variant, sans| {
            openings
                .classify_exact(&play(variant, sans))
                .map(|opening| opening.eco.as_str())
        };
        // Variants that start like standard chess share its names, as long
        // as their extra state is still at its initial value.
        for variant in [
            Variant::Chess,
            Variant::Crazyhouse,
            Variant::ThreeCheck,
            Variant::KingOfTheHill,
        ] {
            assert_eq!(eco(variant, "e4 e5 Nf3"), Some("C44"));
        }
        assert_eq!(eco(Variant::Chess, "e4 d5 exd5 Qxd5"), Some("B01"));
        assert_eq!(eco(Variant::Chess, "e4 f6 Qh5+"), Some("B00"));
        // After captures, crazyhouse pockets are no longer empty.
        assert_eq!(eco(Variant::Crazyhouse, "e4 d5 exd5 Qxd5"), None);
        // After a check, fewer checks remain in three-check.
        assert_eq!(eco(Variant::ThreeCheck, "e4 f6 Qh5+"), None);
        // Other variants have no names at all.
        assert_eq!(eco(Variant::Atomic, "e4 e5 Nf3"), None);
        assert_eq!(eco(Variant::Antichess, "e4 e5 Nf3"), None);
    }
}
