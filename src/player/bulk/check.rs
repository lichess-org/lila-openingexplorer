//! Checks of a bulk-built (and aged) database, for a sample of the players
//! of the mock fixture:
//!
//! * the games of each player are exactly those the live indexer would
//!   have written from the mock up to the player's `latest createdAt`
//!   (same records, same order);
//! * every position of the player's games reads (warm entry plus validated
//!   cold candidates) the same entry as a naive aggregation of the games.

use std::collections::{BTreeMap, HashMap, HashSet};

use shakmaty::{CastlingMode, Color, Position, packed::PackedUciMove, variant::VariantPosition};

use crate::{
    mock_lila::{
        Fixture, FixtureGame, Noise,
        fixture::record_bodies,
        prepare::mix,
        serve::{Presented, present},
        synth::{Synth, user_fingerprint},
    },
    packed::{CellKey, GameIndex, PlayerEntry, PlayerGroupHeader, PositionKey},
    player::{
        MAX_PLY, PlayerDb, UserId,
        game::PlayerGame,
        keys::PlayerGameIndex,
        lila::{Classified, ExportGame},
    },
};

#[derive(Debug, Default)]
pub struct CheckStats {
    pub players: u64,
    pub unindexed: u64,
    pub games: u64,
    pub game_mismatches: u64,
    pub positions: u64,
    pub position_mismatches: u64,
    pub skipped_naive: u64,
}

pub struct CheckConfig<'a> {
    pub fixture: &'a Fixture,
    pub synth: Synth,
    pub noise: Noise,
    pub players: usize,
    /// Players with more games are only checked against the mock.
    pub max_naive_games: u32,
    pub seed: u64,
}

fn pack(mut entry: PlayerEntry) -> Vec<u8> {
    let mut buf = Vec::new();
    entry.pack_entire(&mut buf);
    buf
}

/// The games the live indexer writes for the user from the mock, with
/// `createdAt` (shifted) up to `latest`.
fn expected_games(
    fixture: &Fixture,
    user: &UserId,
    latest: u64,
    config: &CheckConfig<'_>,
) -> std::io::Result<Vec<PlayerGame>> {
    let Some(games) = fixture.player(user) else {
        return Ok(Vec::new());
    };
    let end = fixture.lower_bound(games, config.synth.unshifted(latest).saturating_add(1));
    let bytes = fixture.read_records(games.first, end)?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for body in record_bodies(&bytes) {
        let game = FixtureGame::unpack_body(body).map_err(std::io::Error::other)?;
        let Presented::Line(line) =
            present(&config.synth, &config.noise, &game, user, u64::MAX, false)
        else {
            continue;
        };
        if let Classified::Game { game, .. } =
            ExportGame::parse(&line).expect("mock line").classify(user)
            && seen.insert(game.id)
        {
            out.push(*game);
        }
    }
    Ok(out)
}

/// Naive aggregation of the games, with a position for each key.
fn naive(games: &[PlayerGame]) -> HashMap<(Color, PositionKey), (VariantPosition, PlayerEntry)> {
    let mut entries: HashMap<(Color, PositionKey), (VariantPosition, PlayerEntry)> = HashMap::new();
    for (seq, game) in games.iter().enumerate() {
        let mut pos = game.as_borrowed().initial_position();
        let Ok(mut decoder) = game.moves.decode() else {
            continue;
        };
        let mut last: BTreeMap<PositionKey, (VariantPosition, PackedUciMove)> = BTreeMap::new();
        let mut ply = 0;
        while ply < MAX_PLY
            && let Ok(Some(m)) = decoder.decode_next(&pos)
        {
            last.insert(
                PositionKey::new(&pos),
                (
                    pos.clone(),
                    PackedUciMove::pack(m.to_uci(CastlingMode::Chess960)),
                ),
            );
            pos.play_unchecked(m);
            ply += 1;
        }
        for (key, (pos, m)) in last {
            entries
                .entry((game.color, key))
                .or_insert_with(|| (pos, PlayerEntry::new()))
                .1
                .add_game(
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
                    GameIndex::new(seq as u64).expect("game index"),
                );
        }
    }
    entries
}

pub fn check(
    db: &PlayerDb,
    config: &CheckConfig<'_>,
) -> Result<CheckStats, Box<dyn std::error::Error>> {
    let mut players: Vec<(&UserId, u64)> = config
        .fixture
        .players()
        .filter(|(_, games)| games.count > 0)
        .map(|(id, _)| (id, mix(config.seed ^ user_fingerprint(id))))
        .collect();
    players.sort_by_key(|(_, key)| *key);
    players.truncate(config.players);

    let mut stats = CheckStats::default();
    let reader = db.reader();
    for (user, _) in players {
        stats.players += 1;
        let Some(status) = reader.status(db.user_hash(user))? else {
            stats.unindexed += 1;
            continue;
        };
        let expected = expected_games(config.fixture, user, status.latest_created_at.0, config)?;
        let seqs: Vec<PlayerGameIndex> = (0..status.games).map(PlayerGameIndex).collect();
        let actual: Vec<PlayerGame> = reader
            .games(db.user_hash(user), &seqs)?
            .into_iter()
            .map(|g| g.expect("game of status"))
            .collect();
        stats.games += actual.len() as u64;
        if actual != expected {
            stats.game_mismatches += 1;
            let first = actual.iter().zip(&expected).position(|(a, b)| a != b);
            log::error!(
                "{user}: {} games in the database, {} expected from the mock, first difference at {first:?}: {:?} vs {:?}",
                actual.len(),
                expected.len(),
                first.map(|i| &actual[i]),
                first.map(|i| &expected[i]),
            );
        }
        if status.games > config.max_naive_games {
            stats.skipped_naive += 1;
            continue;
        }
        let mut wrong = 0;
        for ((color, _), (pos, entry)) in naive(&actual) {
            let (read, _) = reader.position(&status, color, &pos)?;
            stats.positions += 1;
            if pack(read) != pack(entry) {
                wrong += 1;
            }
        }
        if wrong > 0 {
            stats.position_mismatches += wrong;
            log::error!("{user}: {wrong} positions differ from the naive aggregation");
        }
        log::debug!("{user}: {} games ok", actual.len());
    }
    Ok(stats)
}
