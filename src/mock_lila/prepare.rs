//! One pass over the parquet that writes a fixture with the games of a set
//! of players.

use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

use shakmaty::{
    Bitboard, ByColor, Color, EnPassantMode, KnownOutcome, Position, Rank, Role,
    fen::Fen,
    san::SanPlus,
    variant::{Variant, VariantPosition},
};

use crate::{
    lila_variant::LilaVariant,
    mock_lila::fixture::{
        FinalStatus, FixtureError, FixtureGame, FixtureMeta, FixturePlayer, FixtureWriter,
    },
    packed::{BorrowedLichessGame, LaxDate, PackedMovesDecoder, Speed},
    parquet::Input,
    player::UserId,
    util::civil::{MS_PER_DAY, days_from_civil},
};

pub fn day_of(date: LaxDate) -> u32 {
    let days = days_from_civil(
        i64::from(date.year()),
        u32::from(date.month().unwrap_or(1)),
        u32::from(date.day().unwrap_or(1)),
    );
    u32::try_from(days.max(0)).expect("day fits")
}

/// Hash of the lowercase bytes of a name, to look up players without
/// allocating.
pub fn name_hash(name: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in name {
        h ^= u64::from(b.to_ascii_lowercase());
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Deterministic pseudo-random number from a few inputs (splitmix64).
pub fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn looks_like_chess960(pos: &VariantPosition) -> bool {
    let board = pos.board();
    let rank = |rank: Rank| Bitboard::from(rank);
    let back_rank_pieces = |role: Role| {
        (board.by_role(role) & board.white() & rank(Rank::First)).count()
            == (board.by_role(role) & board.black() & rank(Rank::Eighth)).count()
    };
    pos.turn() == Color::White
        && pos.fullmoves().get() == 1
        && board.occupied().count() == 32
        && (board.pawns() & board.white() & rank(Rank::Second)).count() == 8
        && (board.pawns() & board.black() & rank(Rank::Seventh)).count() == 8
        && (board.white() & rank(Rank::First)).count() == 8
        && (board.black() & rank(Rank::Eighth)).count() == 8
        && Role::ALL.into_iter().all(back_rank_pieces)
}

/// Converts a parquet game to a fixture game (with `createdAt` 0, set
/// later), replaying it to SAN.
pub fn fixture_game(id: crate::packed::GameId, game: &BorrowedLichessGame<'_>) -> FixtureGame {
    let initial = game.initial_position();
    let variant = match initial.variant() {
        Variant::Chess if game.setup.is_none() => LilaVariant::Standard,
        Variant::Chess if looks_like_chess960(&initial) => LilaVariant::Chess960,
        Variant::Chess => LilaVariant::FromPosition,
        Variant::Antichess => LilaVariant::Antichess,
        Variant::Atomic => LilaVariant::Atomic,
        Variant::Crazyhouse => LilaVariant::Crazyhouse,
        Variant::Horde => LilaVariant::Horde,
        Variant::KingOfTheHill => LilaVariant::KingOfTheHill,
        Variant::RacingKings => LilaVariant::RacingKings,
        Variant::ThreeCheck => LilaVariant::ThreeCheck,
    };
    let initial_fen = game
        .setup
        .is_some()
        .then(|| Fen::from_position(&initial, EnPassantMode::Legal).to_string());

    let mut pos = initial;
    let mut sans = String::with_capacity(game.moves.len() * 5);
    if let Ok(mut decoder) = PackedMovesDecoder::new(game.moves) {
        while let Ok(Some(m)) = decoder.decode_next(&pos) {
            if !sans.is_empty() {
                sans.push(' ');
            }
            let san = SanPlus::from_move_and_play_unchecked(&mut pos, m);
            sans.push_str(&san.to_string());
        }
    }

    let status = if pos.is_checkmate() {
        FinalStatus::Mate
    } else if pos.is_stalemate() {
        FinalStatus::Stalemate
    } else if pos.is_variant_end() {
        FinalStatus::VariantEnd
    } else if game.outcome == KnownOutcome::Draw {
        FinalStatus::Draw
    } else if mix(id.to_bytes().iter().fold(7, |h, &b| h * 31 + u64::from(b))).is_multiple_of(3) {
        FinalStatus::OutOfTime
    } else {
        FinalStatus::Resign
    };

    let player = |color: Color| {
        let p = game.players.get(color);
        FixturePlayer {
            name: p.name.to_owned(),
            title: p.title.map(str::to_owned),
            rating: p
                .rating
                .and_then(|r| u16::try_from(i32::from(r)).ok())
                .filter(|&r| r > 0),
        }
    };

    FixtureGame {
        created_at: 0,
        id,
        variant,
        speed: game.speed.unwrap_or(Speed::Blitz),
        outcome: game.outcome,
        status,
        players: ByColor {
            white: player(Color::White),
            black: player(Color::Black),
        },
        initial_fen,
        sans,
    }
}

/// A selected game of a player, packed in an arena.
struct Rec {
    player: u32,
    row: u64,
    day: u32,
    row_group: u32,
    /// Index of the row among the rows of its day in its row group.
    rank_in_row_group: u32,
    created_at: u64,
    arena: u32,
    offset: u64,
    len: u32,
}

#[derive(Default)]
struct Collected {
    recs: Vec<Rec>,
    arena: Vec<u8>,
    /// Rows of each day in each row group.
    days: HashMap<(u32, u32), u32>,
}

pub struct PrepareConfig<'a> {
    pub input: &'a Input,
    pub players: Vec<UserId>,
    pub out: &'a Path,
    pub threads: usize,
}

/// Writes the fixture of the players.
pub fn prepare(config: PrepareConfig<'_>) -> Result<FixtureMeta, FixtureError> {
    let mut players = config.players;
    players.sort();
    players.dedup();
    let mut lookup: HashMap<u64, Vec<u32>> = HashMap::new();
    for (i, player) in players.iter().enumerate() {
        lookup
            .entry(name_hash(player.as_str().as_bytes()))
            .or_default()
            .push(i as u32);
    }
    let find = |name: &str| -> Option<u32> {
        lookup
            .get(&name_hash(name.as_bytes()))?
            .iter()
            .copied()
            .find(|&i| players[i as usize].is_name(name))
    };

    let input = config.input;
    let n = input.num_row_groups();
    let next = AtomicUsize::new(0);
    let done_rows = AtomicU64::new(0);
    let kept = AtomicU64::new(0);
    let started = Instant::now();
    let results: Mutex<Vec<(usize, Collected)>> = Mutex::new(Vec::new());
    let error: Mutex<Option<io::Error>> = Mutex::new(None);

    thread::scope(|s| {
        for t in 0..config.threads {
            let (next, done_rows, kept, results, error, find) =
                (&next, &done_rows, &kept, &results, &error, &find);
            s.spawn(move || {
                let mut c = Collected::default();
                let mut buf = Vec::new();
                loop {
                    let rg = next.fetch_add(1, Ordering::Relaxed);
                    if rg >= n {
                        break;
                    }
                    let batch = match input.read_games(rg) {
                        Ok(batch) => batch,
                        Err(err) => {
                            *error.lock().expect("error") = Some(io::Error::other(err));
                            break;
                        }
                    };
                    let first = input.first_row(rg);
                    for (i, (id, game)) in batch.rows().enumerate() {
                        let row = first + i as u64;
                        let day = day_of(game.date);
                        let count = c.days.entry((day, rg as u32)).or_insert(0);
                        let rank_in_row_group = *count;
                        *count += 1;
                        let selected: Vec<u32> = Color::ALL
                            .iter()
                            .filter_map(|&color| find(game.players.get(color).name))
                            .collect();
                        if selected.is_empty() {
                            continue;
                        }
                        buf.clear();
                        fixture_game(id, &game).pack(&mut buf);
                        let offset = c.arena.len() as u64;
                        c.arena.extend_from_slice(&buf);
                        for player in selected {
                            c.recs.push(Rec {
                                player,
                                row,
                                day,
                                row_group: rg as u32,
                                rank_in_row_group,
                                created_at: 0,
                                arena: t as u32,
                                offset,
                                len: buf.len() as u32,
                            });
                            kept.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    let d = done_rows.fetch_add(batch.len() as u64, Ordering::Relaxed)
                        + batch.len() as u64;
                    if rg % 1000 == 0 {
                        log::info!(
                            "row group {rg}/{n}, {d} rows, {} kept, {:.0} s",
                            kept.load(Ordering::Relaxed),
                            started.elapsed().as_secs_f64()
                        );
                    }
                }
                results.lock().expect("results").push((t, c));
            });
        }
    });
    if let Some(err) = error.into_inner().expect("error") {
        return Err(err.into());
    }

    let mut collected = results.into_inner().expect("results");
    collected.sort_by_key(|(t, _)| *t);
    // Rank of the first row of a day in each row group among all rows of
    // the day (in dump order), and rows per day. Dumps are ordered by the
    // end of games, so the rows of a day are not contiguous.
    let mut day_counts: Vec<((u32, u32), u32)> = collected
        .iter()
        .flat_map(|(_, c)| c.days.iter().map(|(&k, &v)| (k, v)))
        .collect();
    day_counts.sort_unstable();
    let mut before: HashMap<(u32, u32), u64> = HashMap::with_capacity(day_counts.len());
    let mut totals: HashMap<u32, u64> = HashMap::new();
    for &((day, rg), count) in &day_counts {
        let total = totals.entry(day).or_insert(0);
        before.insert((day, rg), *total);
        *total += u64::from(count);
    }
    let arenas: Vec<Vec<u8>> = collected
        .iter_mut()
        .map(|(_, c)| std::mem::take(&mut c.arena))
        .collect();
    let mut recs: Vec<Rec> = collected.into_iter().flat_map(|(_, c)| c.recs).collect();
    log::info!(
        "pass done in {:.0} s: {} games of {} players, sorting",
        started.elapsed().as_secs_f64(),
        recs.len(),
        players.len()
    );

    // Time of day by the rank of the row among the rows of its day.
    for rec in &mut recs {
        let rank = before[&(rec.day, rec.row_group)] + u64::from(rec.rank_in_row_group);
        let fraction = (rank as f64 + 0.5) / totals[&rec.day] as f64;
        rec.created_at = u64::from(rec.day) * MS_PER_DAY + (fraction * MS_PER_DAY as f64) as u64;
    }
    recs.sort_unstable_by_key(|rec| (rec.player, rec.created_at, rec.row));

    let mut writer = FixtureWriter::create(config.out)?;
    let mut start = 0;
    let mut record = Vec::new();
    for (i, player) in players.iter().enumerate() {
        let end = start + recs[start..].partition_point(|rec| rec.player as usize == i);
        let mut packed = Vec::new();
        let mut previous = None;
        for rec in &recs[start..end] {
            // Strictly increasing per player.
            let created_at = match previous {
                Some(previous) if rec.created_at <= previous => previous + 1,
                _ => rec.created_at,
            };
            previous = Some(created_at);
            record.clear();
            record.extend_from_slice(
                &arenas[rec.arena as usize][rec.offset as usize..][..rec.len as usize],
            );
            record[4..12].copy_from_slice(&created_at.to_be_bytes());
            packed.push(record.clone());
        }
        writer.write_player(player, packed.iter().map(Vec::as_slice))?;
        start = end;
    }
    let meta = writer.finish()?;
    log::info!(
        "wrote {} games of {} players in {:.0} s",
        meta.games,
        meta.players,
        started.elapsed().as_secs_f64()
    );
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chess960() {
        let pos = |fen: &str| {
            VariantPosition::from_setup(
                Variant::Chess,
                fen.parse::<Fen>().unwrap().into_setup(),
                shakmaty::CastlingMode::Chess960,
            )
            .unwrap()
        };
        assert!(looks_like_chess960(&pos(
            "bbnrnkrq/pppppppp/8/8/8/8/PPPPPPPP/BBNRNKRQ w KQkq - 0 1"
        )));
        assert!(!looks_like_chess960(&pos(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
        )));
    }
}
