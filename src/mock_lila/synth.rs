//! Deterministic rules of the mock lila for what the parquet lacks: the
//! time shift, game durations and casual games. Shared with the bulk build
//! of the personal explorer, so that a database built offline agrees with
//! what the mock serves.

use sha2::{Digest, Sha256};

use crate::{
    mock_lila::prepare::mix,
    packed::{GameId, Speed},
    player::UserId,
    util::civil::{civil_from_days, days_from_civil},
};

/// Milliseconds of the first day of the month after the month of `ms`.
pub fn next_month_start(ms: u64) -> u64 {
    let days = (ms / 86_400_000) as i64;
    let (mut year, mut month) = civil_from_days(days);
    month += 1;
    if month > 12 {
        month = 1;
        year += 1;
    }
    days_from_civil(year, month, 1) as u64 * 86_400_000
}

/// Milliseconds per ply, by speed, for synthesized game durations.
fn ms_per_ply(speed: Speed) -> u64 {
    match speed {
        Speed::UltraBullet => 400,
        Speed::Bullet => 1_800,
        Speed::Blitz => 5_000,
        Speed::Rapid => 13_000,
        Speed::Classical => 35_000,
        Speed::Correspondence => 4 * 3_600_000,
    }
}

/// A stable, public hash of a user id, for the deterministic pseudo-random
/// decisions of the mock (and of the bulk build, so that they agree): the
/// first 8 bytes of the SHA-256 digest.
pub fn user_fingerprint(user: &UserId) -> u64 {
    let digest = Sha256::digest(user.as_str().as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("8 bytes"))
}

/// A uniform number in `0.0..1.0` from the inputs.
pub fn unit(seed: u64, a: u64, salt: u64) -> f64 {
    (mix(seed ^ mix(a ^ mix(salt))) >> 11) as f64 / (1u64 << 53) as f64
}

/// The game id as a number, for the pseudo-random choices per game.
pub fn game_number(id: GameId) -> u64 {
    let mut b = [0; 8];
    b[2..].copy_from_slice(&id.to_bytes());
    u64::from_be_bytes(b)
}

#[derive(Debug, Clone)]
pub struct Synth {
    pub seed: u64,
    /// Share of games that are casual.
    pub casual: f64,
    /// Added to `createdAt` of the fixture.
    pub shift: i64,
}

impl Synth {
    /// The shift that moves the newest month of data with the given latest
    /// `createdAt` to end at `data_end`.
    pub fn shift_for(max_created_at: u64, data_end: u64) -> i64 {
        // An hour of slack, for times bumped past midnight to keep them
        // strictly increasing per player.
        let newest = max_created_at.saturating_sub(3_600_000);
        data_end as i64 - next_month_start(newest) as i64
    }

    pub fn shifted(&self, created_at: u64) -> u64 {
        created_at.saturating_add_signed(self.shift)
    }

    pub fn unshifted(&self, created_at: u64) -> u64 {
        created_at.saturating_add_signed(-self.shift)
    }

    /// Duration of a game (ms), from its number of plies.
    pub fn duration(&self, id: u64, plies: u64, speed: Speed) -> u64 {
        (plies * ms_per_ply(speed)).max(10_000) + (unit(self.seed, id, 1) * 10_000.0) as u64
    }

    pub fn rated(&self, id: u64) -> bool {
        unit(self.seed, id, 8) >= self.casual
    }
}
