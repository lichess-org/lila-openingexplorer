use std::{
    collections::HashSet,
    sync::RwLock,
    time::{Duration, SystemTime},
};

/// Where to get the blacklist from.
#[derive(Debug, Clone)]
pub struct LilaConfig {
    /// Base url of the lila instance, like `https://lichess.org`.
    pub url: String,
    /// Token of https://lichess.org/@/OpeningExplorer, for the internal
    /// endpoint.
    pub bearer: Option<String>,
    /// Time between updates.
    pub refresh: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum BlacklistError {
    #[error("request: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid user name in response: {0:?}")]
    InvalidName(String),
}

/// Set of user ids (lowercase user names).
#[derive(Default)]
pub struct Blacklist {
    users: RwLock<HashSet<Box<str>>>,
}

impl Blacklist {
    pub fn len(&self) -> usize {
        self.users.read().expect("read blacklist").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether any of the given players (by name, as in game records) is
    /// blacklisted.
    pub fn contains_any<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> bool {
        let users = self.users.read().expect("read blacklist");
        !users.is_empty()
            && names
                .into_iter()
                .any(|name| users.contains(name.to_ascii_lowercase().as_str()))
    }

    /// Adds users, returning how many were new.
    pub fn extend(&self, names: Vec<Box<str>>) -> usize {
        let mut users = self.users.write().expect("write blacklist");
        let before = users.len();
        users.extend(names);
        users.len() - before
    }
}

fn user_id(line: &[u8]) -> Result<Option<Box<str>>, BlacklistError> {
    let name = line.trim_ascii();
    if name.is_empty() {
        return Ok(None);
    }
    if name.len() > 30
        || !name
            .iter()
            .all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(BlacklistError::InvalidName(
            String::from_utf8_lossy(name).into_owned(),
        ));
    }
    let id = std::str::from_utf8(name)
        .expect("ascii")
        .to_ascii_lowercase();
    Ok(Some(id.into_boxed_str()))
}

/// Streams the users marked since the given time, one name per line, from
/// lila's `/api/stream/mod-marked-since`.
///
/// On error, the users received so far are returned along with the error,
/// so that they can be used anyway.
pub async fn fetch_marked_since(
    client: &reqwest::Client,
    config: &LilaConfig,
    since: SystemTime,
) -> (Vec<Box<str>>, Result<(), BlacklistError>) {
    let mut users = Vec::new();
    let result = async {
        let since = since
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let mut request = client.get(format!(
            "{}/api/stream/mod-marked-since?since={since}",
            config.url
        ));
        if let Some(bearer) = &config.bearer {
            request = request.bearer_auth(bearer);
        }
        let mut response = request.send().await?.error_for_status()?;
        let mut pending = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            pending.extend_from_slice(&chunk);
            let complete = pending
                .iter()
                .rposition(|&c| c == b'\n')
                .map_or(0, |i| i + 1);
            for line in pending[..complete].split(|&c| c == b'\n') {
                users.extend(user_id(line)?);
            }
            pending.drain(..complete);
        }
        users.extend(user_id(&pending)?);
        Ok(())
    }
    .await;
    (users, result)
}

/// Keeps the blacklist up to date, calling `on_change` after users were
/// added (to invalidate cached responses).
pub async fn periodic_update(blacklist: &Blacklist, config: LilaConfig, on_change: impl Fn()) {
    let client = reqwest::Client::builder()
        .user_agent("lila-openingexplorer/4")
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(60))
        .build()
        .expect("reqwest client");

    let mut last_update = None;
    loop {
        let started = SystemTime::now();
        // Overlap with the previous update, for users marked while it ran
        // and clock differences.
        let since = last_update.map_or(SystemTime::UNIX_EPOCH, |last: SystemTime| {
            last.checked_sub(Duration::from_secs(60 * 10))
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });
        let (users, result) = fetch_marked_since(&client, &config, since).await;
        let received = users.len();
        let added = blacklist.extend(users);
        if added > 0 {
            on_change();
        }
        match result {
            Ok(()) => {
                log::info!(
                    "blacklist updated in {:.3?}: {} received, {} new, {} total",
                    started.elapsed().unwrap_or_default(),
                    received,
                    added,
                    blacklist.len()
                );
                last_update = Some(started);
                tokio::time::sleep(config.refresh).await;
            }
            Err(err) => {
                log::error!(
                    "blacklist update failed after {received} users ({added} new, {} total): {err}",
                    blacklist.len()
                );
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_id() {
        assert_eq!(
            user_id(b"  Foo_Bar-1\r").unwrap().as_deref(),
            Some("foo_bar-1")
        );
        assert_eq!(user_id(b"").unwrap(), None);
        assert!(user_id(b"a b").is_err());
    }

    #[test]
    fn test_contains_any() {
        let blacklist = Blacklist::default();
        assert!(!blacklist.contains_any(["foo"]));
        assert_eq!(blacklist.extend(vec!["foo".into(), "bar".into()]), 2);
        assert_eq!(blacklist.extend(vec!["foo".into()]), 0);
        assert!(blacklist.contains_any(["baz", "FoO"]));
        assert!(!blacklist.contains_any(["baz", "qux"]));
    }
}
