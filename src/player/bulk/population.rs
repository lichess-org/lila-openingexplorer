//! The population of the prod-like database: game counts of all players of
//! the parquet, and the selection of players to index.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufReader, BufWriter, Read, Write},
    path::Path,
};

use shakmaty::Color;

use crate::{
    mock_lila::prepare::{mix, name_hash},
    parquet::Input,
    player::bulk::for_each_row_group,
};

/// Games of a player, by the hash of the lowercase name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlayerCount {
    pub name_hash: u64,
    pub games: u32,
}

/// Counts the games of all players of the input.
pub fn count_games(input: &Input, threads: usize) -> io::Result<Vec<PlayerCount>> {
    let maps = for_each_row_group(
        input,
        threads,
        HashMap::<u64, u32>::new,
        |map, _rg, batch| {
            for (_, game) in batch.rows() {
                for color in Color::ALL {
                    *map.entry(name_hash(game.players.get(color).name.as_bytes()))
                        .or_default() += 1;
                }
            }
        },
    )?;
    let mut total: HashMap<u64, u32> = HashMap::new();
    for map in maps {
        if total.is_empty() {
            total = map;
            continue;
        }
        for (k, v) in map {
            *total.entry(k).or_default() += v;
        }
    }
    let mut counts: Vec<PlayerCount> = total
        .into_iter()
        .map(|(name_hash, games)| PlayerCount { name_hash, games })
        .collect();
    counts.sort_unstable();
    Ok(counts)
}

/// ```text
/// counts := (u64(name hash) u32(games))*   (little endian, by hash)
/// ```
pub fn write_counts(path: &Path, counts: &[PlayerCount]) -> io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    for c in counts {
        w.write_all(&c.name_hash.to_le_bytes())?;
        w.write_all(&c.games.to_le_bytes())?;
    }
    w.flush()
}

pub fn read_counts(path: &Path) -> io::Result<Vec<PlayerCount>> {
    let mut bytes = Vec::new();
    BufReader::new(File::open(path)?).read_to_end(&mut bytes)?;
    Ok(bytes
        .as_chunks::<12>()
        .0
        .iter()
        .map(|c| PlayerCount {
            name_hash: u64::from_le_bytes(c[..8].try_into().expect("8 bytes")),
            games: u32::from_le_bytes(c[8..].try_into().expect("4 bytes")),
        })
        .collect())
}

/// Why a player is in the population.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// A player of `requests.tsv` (in the mock fixture).
    Requests = 0,
    /// Sampled to reach the target size.
    Sample = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Member {
    pub name_hash: u64,
    pub games: u32,
    pub origin: Origin,
}

#[derive(Debug, Clone)]
pub struct SelectConfig {
    /// Players in the population, including the requests players.
    pub players: usize,
    /// Mean games per sampled player.
    pub mean_games: f64,
    /// Sampled players have at least this many games.
    pub min_games: u32,
    pub seed: u64,
}

/// Selects the requests players and a sample of the others, with games
/// counts drawn so that the sample has the given mean: players are sampled
/// with a weight `games^alpha`, with `alpha` found by bisection.
pub fn select(
    counts: &[PlayerCount],
    requests: &[u64],
    config: &SelectConfig,
) -> (Vec<Member>, f64) {
    let requests: std::collections::HashSet<u64> = requests.iter().copied().collect();
    let mut members: Vec<Member> = counts
        .iter()
        .filter(|c| requests.contains(&c.name_hash))
        .map(|c| Member {
            name_hash: c.name_hash,
            games: c.games,
            origin: Origin::Requests,
        })
        .collect();
    let candidates: Vec<&PlayerCount> = counts
        .iter()
        .filter(|c| c.games >= config.min_games && !requests.contains(&c.name_hash))
        .collect();
    let wanted = config.players.saturating_sub(members.len());

    // Weighted sampling without replacement by exponential keys
    // (Efraimidis-Spirakis): key = u^(1/w), take the largest. With
    // w = games^alpha: log key = ln(u) / games^alpha.
    let sample = |alpha: f64| -> Vec<&PlayerCount> {
        let mut keyed: Vec<(f64, &PlayerCount)> = candidates
            .iter()
            .map(|c| {
                let u = ((mix(config.seed ^ c.name_hash) >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
                (u.ln() / f64::from(c.games).powf(alpha), *c)
            })
            .collect();
        let wanted = wanted.min(keyed.len());
        if wanted > 0 && wanted < keyed.len() {
            keyed.select_nth_unstable_by(wanted - 1, |a, b| b.0.total_cmp(&a.0));
        }
        keyed.truncate(wanted);
        keyed.into_iter().map(|(_, c)| c).collect()
    };
    let mean = |s: &[&PlayerCount]| {
        s.iter().map(|c| f64::from(c.games)).sum::<f64>() / s.len().max(1) as f64
    };
    let (mut lo, mut hi) = (-2.0f64, 2.0f64);
    let mut chosen = sample(0.0);
    let mut alpha = 0.0;
    for _ in 0..30 {
        alpha = (lo + hi) / 2.0;
        chosen = sample(alpha);
        if mean(&chosen) < config.mean_games {
            lo = alpha;
        } else {
            hi = alpha;
        }
        if (mean(&chosen) - config.mean_games).abs() < 0.001 * config.mean_games {
            break;
        }
    }
    members.extend(chosen.into_iter().map(|c| Member {
        name_hash: c.name_hash,
        games: c.games,
        origin: Origin::Sample,
    }));
    members.sort_unstable();
    (members, alpha)
}

/// ```text
/// population := (u64(name hash) u32(games) u8(origin))*   (by hash)
/// ```
pub fn write_population(path: &Path, members: &[Member]) -> io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    for m in members {
        w.write_all(&m.name_hash.to_le_bytes())?;
        w.write_all(&m.games.to_le_bytes())?;
        w.write_all(&[m.origin as u8])?;
    }
    w.flush()
}

pub fn read_population(path: &Path) -> io::Result<Vec<Member>> {
    let mut bytes = Vec::new();
    BufReader::new(File::open(path)?).read_to_end(&mut bytes)?;
    Ok(bytes
        .as_chunks::<13>()
        .0
        .iter()
        .map(|c| Member {
            name_hash: u64::from_le_bytes(c[..8].try_into().expect("8 bytes")),
            games: u32::from_le_bytes(c[8..12].try_into().expect("4 bytes")),
            origin: if c[12] == 0 {
                Origin::Requests
            } else {
                Origin::Sample
            },
        })
        .collect())
}
