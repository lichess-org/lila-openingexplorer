//! Storage tests: whatever the chunk boundaries and rebuild points, reads
//! give the same entries as a naive aggregation. Also of the placement into
//! warm entries and cold groups alone, with many more games.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use arbitrary::Unstructured;
use arbtest::arbtest;
use shakmaty::{
    CastlingMode, Chess, Color, EnPassantMode, Position,
    packed::PackedUciMove,
    variant::{Variant, VariantPosition},
};

use crate::{
    packed::{
        CellKey, ColdGroupView, GameId, GameIndex, Month, PackedMoves, PlayerEntry,
        PlayerGroupHeader, PositionKey, Rating, RatingMode, Speed,
    },
    player::{
        MAX_PLY, PlayerDb, PlayerDbConfig, PlayerDbError, UserHasher, UserId,
        aggregate::{Aggregation, find_visit},
        game::PlayerGame,
        keys::{ColdBucket, PlayerGameIndex},
        status::PlayerStatus,
    },
};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "player-{name}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A random game with a few likely openings, so that positions repeat
/// across games.
pub(super) fn arbitrary_game(
    u: &mut Unstructured<'_>,
    n: u64,
) -> arbitrary::Result<(PlayerGame, Vec<VariantPosition>)> {
    let mut pos = if u.ratio(1, 20)? {
        VariantPosition::new(Variant::Atomic)
    } else if u.ratio(1, 20)? {
        VariantPosition::from_setup(
            Variant::Chess,
            "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2"
                .parse::<shakmaty::fen::Fen>()
                .unwrap()
                .into_setup(),
            CastlingMode::Chess960,
        )
        .unwrap()
    } else {
        VariantPosition::Chess(Chess::default())
    };
    let setup = PlayerGame::setup_of(&pos);
    let mut encoder = PackedMoves::encoder();
    let mut positions = vec![pos.clone()];
    let plies = u.int_in_range(0..=60)?;
    for ply in 0..plies {
        let legals = pos.legal_moves();
        if legals.is_empty() {
            break;
        }
        // Mostly the first moves early on, to have common positions, and
        // sometimes back and forth, for loops.
        let m = if ply < 6 && u.ratio(3, 4)? {
            legals[usize::from(u.int_in_range(0..=1u8)?).min(legals.len() - 1)]
        } else {
            *u.choose(&legals)?
        };
        encoder.encode_next(&pos, m);
        pos.play_unchecked(m);
        positions.push(pos.clone());
    }
    let mut id = [0u8; 8];
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut x = n;
    for c in id.iter_mut() {
        *c = digits[(x % 62) as usize];
        x /= 62;
    }
    Ok((
        PlayerGame {
            id: GameId::from_ascii(&id).unwrap(),
            speed: Speed::from_bits(u.int_in_range(1..=6)?).unwrap(),
            mode: if u.arbitrary()? {
                RatingMode::Rated
            } else {
                RatingMode::Casual
            },
            month: Month::from_months_since_jan_1900(u.int_in_range(1400..=1404)?),
            outcome: u.arbitrary()?,
            color: if u.arbitrary()? {
                Color::White
            } else {
                Color::Black
            },
            player_rating: Rating::try_from(u.int_in_range(800..=2800)?).unwrap(),
            opponent_rating: Rating::try_from(u.int_in_range(800..=2800)?).unwrap(),
            opponent_name: "opponent".to_owned(),
            opponent_title: None,
            player_title: None,
            setup,
            moves: encoder.finish(),
        },
        positions,
    ))
}

/// Naive aggregation: replay every game, keep the last move from each
/// position within the first `MAX_PLY` plies.
fn naive(
    games: &[(PlayerGame, Vec<VariantPosition>)],
) -> HashMap<(Color, PositionKey), PlayerEntry> {
    let mut entries: HashMap<(Color, PositionKey), PlayerEntry> = HashMap::new();
    for (index, (game, positions)) in games.iter().enumerate() {
        let mut pos = game.as_borrowed().initial_position();
        let mut decoder = game.moves.decode().unwrap();
        let mut last: BTreeMap<PositionKey, PackedUciMove> = BTreeMap::new();
        let mut ply = 0;
        while ply < MAX_PLY
            && let Some(m) = decoder.decode_next(&pos).unwrap()
        {
            assert_eq!(pos, positions[ply]);
            last.insert(
                PositionKey::new(&pos),
                PackedUciMove::pack(m.to_uci(CastlingMode::Chess960)),
            );
            pos.play_unchecked(m);
            ply += 1;
        }
        for (key, m) in last {
            entries.entry((game.color, key)).or_default().add_game(
                CellKey::new(
                    m,
                    PlayerGroupHeader {
                        speed: game.speed,
                        mode: game.mode,
                    },
                    game.month,
                ),
                game.opponent_rating,
                game.outcome,
                GameIndex::new(index as u64).unwrap(),
            );
        }
    }
    entries
}

fn pack(mut entry: PlayerEntry) -> Vec<u8> {
    let mut buf = Vec::new();
    entry.pack_entire(&mut buf);
    buf
}

#[test]
fn test_storage_matches_naive() {
    arbtest(|u| {
        let dir = TempDir::new("storage");
        let db = PlayerDb::open(
            &dir.0,
            PlayerDbConfig {
                cache_bytes: 8 << 20,
                // Sometimes ingest even small rebuilds.
                ingest_bytes: if u.arbitrary()? { 0 } else { 64 << 20 },
                batch_bytes: u.int_in_range(100..=10_000)?,
                ..PlayerDbConfig::default()
            },
        )
        .unwrap();

        let users = [
            UserId::from_name("alice").unwrap(),
            UserId::from_name("bob").unwrap(),
        ];
        let mut games: [Vec<(PlayerGame, Vec<VariantPosition>)>; 2] = [Vec::new(), Vec::new()];
        let mut statuses = users.clone().map(PlayerStatus::new);
        let mut n = 0;
        for _ in 0..u.int_in_range(0..=8)? {
            let player = usize::from(u.ratio(1, 4)?);
            let mut chunk = Vec::new();
            for _ in 0..u.int_in_range(0..=30)? {
                // Sometimes a game that is already indexed again.
                if u.ratio(1, 10)?
                    && let Some((game, _)) = u.choose(&games[player]).ok().cloned()
                {
                    chunk.push(game);
                    continue;
                }
                let game = arbitrary_game(u, n)?;
                n += 1;
                chunk.push(game.0.clone());
                games[player].push(game);
            }
            let stats = db.write_chunk(&mut statuses[player], &chunk).unwrap();
            assert_eq!(stats.games + stats.known, chunk.len() as u64);
            if u.ratio(1, 3)? {
                db.rebuild(db.user_hash(&users[player])).unwrap();
                statuses[player] = db.status(db.user_hash(&users[player])).unwrap().unwrap();
            }
            if u.ratio(1, 4)? {
                db.flush().unwrap();
            }
        }
        if u.arbitrary()? {
            db.compact();
        }

        let reader = db.reader();
        for (player, user) in users.iter().enumerate() {
            let expected = naive(&games[player]);
            let Some(status) = reader.status(db.user_hash(user)).unwrap() else {
                assert!(games[player].is_empty());
                continue;
            };
            assert_eq!(status.games as usize, games[player].len());
            // Rebuilds leave nothing of older generations.
            let generations = db.index_generations(db.user_hash(user)).unwrap();
            assert!(
                generations.iter().all(|&g| g == status.generation),
                "{generations:?} besides {:?}",
                status.generation
            );
            let mut probes: Vec<(Color, VariantPosition)> = games[player]
                .iter()
                .flat_map(|(game, positions)| positions.iter().map(|pos| (game.color, pos.clone())))
                .collect();
            probes.push((Color::White, VariantPosition::new(Variant::Chess)));
            probes.push((Color::Black, VariantPosition::new(Variant::Chess)));
            for (color, pos) in probes {
                let key = PositionKey::new(&pos);
                let (entry, _) = reader.position(&status, color, &pos).unwrap();
                let expected = expected.get(&(color, key)).cloned().unwrap_or_default();
                assert_eq!(
                    pack(entry),
                    pack(expected),
                    "{user} {color} {}",
                    shakmaty::fen::Fen::from_position(&pos, EnPassantMode::Legal)
                );
            }
        }
        Ok(())
    })
    .budget_ms(10_000);
}

/// Reading the placement like the database does (the warm entry and the
/// cold candidates that replay confirms) gives the naive entries, with
/// enough games to have positions of a game that share a cold item, one of
/// them warm.
#[test]
fn test_placement_matches_naive() {
    arbtest(|u| {
        let bits = u.int_in_range(0..=2)?;
        // Plenty of data for the games, from a seed.
        let mut x: u64 = u.arbitrary()?;
        let data: Vec<u8> = (0..1 << 20)
            .map(|_| {
                x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = x;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                (z ^ (z >> 31)) as u8
            })
            .collect();
        let mut games_data = Unstructured::new(&data);
        let games: Vec<(PlayerGame, Vec<VariantPosition>)> = (0..2000)
            .map(|n| arbitrary_game(&mut games_data, n))
            .collect::<arbitrary::Result<_>>()?;

        let mut aggregation = Aggregation::new();
        for (index, (game, _)) in games.iter().enumerate() {
            aggregation.add_game(PlayerGameIndex(index as u32), &game.as_borrowed());
        }
        let placed = aggregation.place(|_, _| bits);
        assert_eq!(placed.cold_bits, shakmaty::ByColor::new_with(|_| bits));
        let warm: HashMap<(Color, PositionKey), PlayerEntry> = placed
            .warm
            .into_iter()
            .map(|(color, key, entry)| ((color, key), entry))
            .collect();
        let cold: HashMap<(Color, ColdBucket), Vec<u8>> = placed
            .cold
            .into_iter()
            .map(|(color, bucket, mut group)| {
                let mut buf = Vec::new();
                group.pack_entire(&mut buf);
                ((color, bucket), buf)
            })
            .collect();

        for ((color, key), expected) in naive(&games) {
            let mut entry = warm.get(&(color, key)).cloned().unwrap_or_default();
            let (bucket, suffix) = ColdBucket::of(&key, bits);
            if let Some(group) = cold.get(&(color, bucket)) {
                for index in ColdGroupView::unpack_entire(group).candidates(suffix) {
                    let game = games[index as usize].0.as_borrowed();
                    assert_eq!(game.color, color);
                    if let Some(m) = find_visit(&game, &key, MAX_PLY) {
                        entry.add_game(
                            CellKey::new(
                                m,
                                PlayerGroupHeader {
                                    speed: game.speed,
                                    mode: game.mode,
                                },
                                game.month,
                            ),
                            game.opponent_rating,
                            game.outcome,
                            GameIndex::new(index).unwrap(),
                        );
                    }
                }
            }
            assert_eq!(pack(entry), pack(expected), "{color} {key:?}");
        }
        Ok(())
    })
    .budget_ms(3_000);
}

#[test]
fn test_user_hash_key() {
    let dir = TempDir::new("user_hash_key");
    let alice = UserId::from_name("alice").unwrap();
    let given = UserHasher::from_key([5; UserHasher::KEY_SIZE]);
    let config = |user_hasher| PlayerDbConfig {
        user_hasher,
        ..PlayerDbConfig::default()
    };
    let generated = {
        let db = PlayerDb::open(&dir.0.join("generated"), config(None)).unwrap();
        let mut status = PlayerStatus::new(alice.clone());
        let mut u = Unstructured::new(&[7; 4096]);
        let (game, _) = arbitrary_game(&mut u, 1).unwrap();
        db.write_chunk(&mut status, &[game]).unwrap();
        db.user_hash(&alice)
    };
    assert_ne!(generated, given.hash(&alice));
    let db = PlayerDb::open(&dir.0.join("generated"), config(None)).unwrap();
    assert_eq!(db.user_hash(&alice), generated);
    assert!(db.status(generated).unwrap().is_some());
    drop(db);
    assert!(matches!(
        PlayerDb::open(&dir.0.join("generated"), config(Some(given.clone()))),
        Err(PlayerDbError::UserHashKeyMismatch)
    ));

    let db = PlayerDb::open(&dir.0.join("given"), config(Some(given.clone()))).unwrap();
    assert_eq!(db.user_hash(&alice), given.hash(&alice));
    drop(db);
    let db = PlayerDb::open(&dir.0.join("given"), config(None)).unwrap();
    assert_eq!(db.user_hash(&alice), given.hash(&alice));
}
