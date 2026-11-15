use std::time::Duration;

use crate::player::{status::PlayerStatus, time::UnixMillis};

/// When to index a player.
#[derive(Debug, Clone)]
pub struct RunPolicy {
    /// No new index run within this time after the last one.
    pub cooldown: Duration,
    /// Ongoing games are revisited at most this often.
    pub revisit_interval: Duration,
}

impl Default for RunPolicy {
    fn default() -> RunPolicy {
        RunPolicy {
            cooldown: Duration::from_secs(2 * 60),
            revisit_interval: Duration::from_secs(15 * 60),
        }
    }
}

/// An index run: request games created since a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRun {
    /// Request games with `createdAt >= since`.
    pub since: UnixMillis,
    /// Revisits ongoing games (and also gets all newer games).
    pub revisit: bool,
}

impl RunPolicy {
    /// The run to do now, if any.
    pub fn next_run(&self, status: Option<&PlayerStatus>, now: UnixMillis) -> Option<IndexRun> {
        let Some(status) = status else {
            return Some(IndexRun {
                since: UnixMillis::EPOCH,
                revisit: false,
            });
        };
        if now.elapsed_since(status.indexed_at) < self.cooldown {
            return None;
        }
        if let Some(ongoing) = status.revisit_ongoing
            && now.elapsed_since(status.revisited_at) >= self.revisit_interval
        {
            return Some(IndexRun {
                since: ongoing,
                revisit: true,
            });
        }
        Some(IndexRun {
            // Including the latest millisecond, for games created in the
            // same millisecond after the last run. The games received again
            // are known and skipped.
            since: status.latest_created_at,
            revisit: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::UserId;

    #[test]
    fn test_next_run() {
        let policy = RunPolicy::default();
        let alice = UserId::from_name("alice").unwrap();
        let run = |since, revisit| {
            Some(IndexRun {
                since: UnixMillis(since),
                revisit,
            })
        };
        assert_eq!(policy.next_run(None, UnixMillis(0)), run(0, false));
        let mut status = PlayerStatus::new(alice);
        status.games = 10;
        status.latest_created_at = UnixMillis(1000);
        let t = 100_000_000_000;
        status.indexed_at = UnixMillis(t);
        assert_eq!(policy.next_run(Some(&status), UnixMillis(t + 60_000)), None);
        assert_eq!(
            policy.next_run(Some(&status), UnixMillis(t + 120_000)),
            run(1000, false)
        );
        status.revisit_ongoing = Some(UnixMillis(500));
        assert_eq!(
            policy.next_run(Some(&status), UnixMillis(t + 120_000)),
            run(500, true)
        );
        status.revisited_at = UnixMillis(t);
        assert_eq!(
            policy.next_run(Some(&status), UnixMillis(t + 120_000)),
            run(1000, false)
        );
    }
}
