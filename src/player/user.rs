use std::{fmt, str::FromStr};

use arbitrary::{Arbitrary, Unstructured};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(Box<str>);

#[derive(Debug, Error)]
#[error("invalid user name")]
pub struct InvalidUserName;

impl UserId {
    pub const MAX_LEN: usize = 30;

    /// The id of a user name (case-insensitive): 1 to 30 ASCII letters,
    /// digits, `_` and `-`.
    pub fn from_name(name: &str) -> Result<UserId, InvalidUserName> {
        if name.is_empty()
            || name.len() > UserId::MAX_LEN
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err(InvalidUserName);
        }
        Ok(UserId(name.to_ascii_lowercase().into_boxed_str()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the name (as in game records) is the name of this user.
    pub fn is_name(&self, name: &str) -> bool {
        self.0.eq_ignore_ascii_case(name)
    }
}

impl Arbitrary<'_> for UserId {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<UserId> {
        const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_-";
        let len = u.int_in_range(1..=UserId::MAX_LEN)?;
        let name = (0..len)
            .map(|_| u.choose(CHARS).map(|&c| char::from(c)))
            .collect::<arbitrary::Result<String>>()?;
        Ok(UserId(name.into_boxed_str()))
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for UserId {
    type Err = InvalidUserName;

    fn from_str(s: &str) -> Result<UserId, InvalidUserName> {
        UserId::from_name(s)
    }
}

/// Keyed hash of a user id.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserHash([u8; UserHash::SIZE]);

impl UserHash {
    pub const SIZE: usize = 8;

    pub fn from_bytes(bytes: [u8; UserHash::SIZE]) -> UserHash {
        UserHash(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; UserHash::SIZE] {
        &self.0
    }

    pub fn to_u64(self) -> u64 {
        u64::from_be_bytes(self.0)
    }
}

#[derive(Clone)]
pub struct UserHasher {
    key: [u8; UserHasher::KEY_SIZE],
    mac: Hmac<Sha256>,
}

impl UserHasher {
    pub const KEY_SIZE: usize = 32;

    pub fn from_key(key: [u8; UserHasher::KEY_SIZE]) -> UserHasher {
        UserHasher {
            key,
            mac: Hmac::new_from_slice(&key).expect("any key size"),
        }
    }

    pub fn generate() -> UserHasher {
        let mut key = [0; UserHasher::KEY_SIZE];
        getrandom::fill(&mut key).expect("random key");
        UserHasher::from_key(key)
    }

    pub fn key(&self) -> &[u8; UserHasher::KEY_SIZE] {
        &self.key
    }

    pub fn from_key_bytes(bytes: &[u8]) -> Option<UserHasher> {
        bytes.try_into().ok().map(UserHasher::from_key)
    }

    pub fn hash(&self, user: &UserId) -> UserHash {
        let mut mac = self.mac.clone();
        mac.update(user.as_str().as_bytes());
        let digest = mac.finalize().into_bytes();
        UserHash(digest[..UserHash::SIZE].try_into().expect("8 bytes"))
    }
}

impl fmt::Display for UserHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for UserHasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserHasher").finish_non_exhaustive()
    }
}

impl PartialEq for UserHasher {
    fn eq(&self, other: &UserHasher) -> bool {
        self.key == other.key
    }
}

impl Eq for UserHasher {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_id() {
        let id = UserId::from_name("Foo_Bar-1").unwrap();
        assert_eq!(id.as_str(), "foo_bar-1");
        assert!(id.is_name("FOO_bar-1"));
        assert!(UserId::from_name("").is_err());
        assert!(UserId::from_name("a b").is_err());
        assert!(UserId::from_name(&"a".repeat(31)).is_err());
    }

    #[test]
    fn test_user_hash() {
        let hasher = UserHasher::from_key([7; UserHasher::KEY_SIZE]);
        let id = UserId::from_name("foo_bar-1").unwrap();
        assert_eq!(
            hasher.hash(&id),
            hasher.hash(&UserId::from_name("FOO_bar-1").unwrap())
        );
        assert_ne!(
            hasher.hash(&id),
            hasher.hash(&UserId::from_name("foo_bar-2").unwrap())
        );
        assert_ne!(hasher.hash(&id), UserHasher::generate().hash(&id));
    }

    #[test]
    fn test_user_hash_is_stable() {
        // Part of the database format: must never change. Python:
        // hmac.new(bytes([7] * 32), b"revoof", hashlib.sha256).
        let hasher = UserHasher::from_key([7; UserHasher::KEY_SIZE]);
        assert_eq!(
            hasher.hash(&UserId::from_name("revoof").unwrap()).to_u64(),
            0x2d4b_f6b0_e9f5_e360
        );
    }
}
