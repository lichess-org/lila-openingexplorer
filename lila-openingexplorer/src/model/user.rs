use std::{fmt, str::FromStr};

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct UserName(String);

impl UserName {
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<UserName, InvalidUserName> {
        if !bytes.is_empty()
            && bytes.len() <= 30
            && bytes
                .iter()
                .copied()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            Ok(UserName(
                String::from_utf8(bytes.to_owned()).map_err(|_| InvalidUserName)?,
            ))
        } else {
            Err(InvalidUserName)
        }
    }
}

impl fmt::Display for UserName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Error, Debug)]
#[error("invalid username")]
pub struct InvalidUserName;

impl FromStr for UserName {
    type Err = InvalidUserName;

    fn from_str(s: &str) -> Result<UserName, InvalidUserName> {
        UserName::from_bytes(s.as_bytes())
    }
}

impl PartialEq for UserName {
    fn eq(&self, other: &UserName) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl PartialEq<UserId> for UserName {
    fn eq(&self, other: &UserId) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl Eq for UserName {}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct UserId(String);

impl From<UserName> for UserId {
    fn from(UserName(mut name): UserName) -> UserId {
        name.make_ascii_lowercase();
        UserId(name)
    }
}

impl UserId {
    pub fn as_lowercase_str(&self) -> &str {
        &self.0
    }
}

impl PartialEq<UserName> for UserId {
    fn eq(&self, other: &UserName) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}
