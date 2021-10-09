use std::error::Error as StdError;
use std::fmt;
use std::str::FromStr;

#[derive(Debug)]
pub struct UserName(String);

impl fmt::Display for UserName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug)]
pub struct InvalidUserName;

impl fmt::Display for InvalidUserName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid username")
    }
}

impl StdError for InvalidUserName {}

impl FromStr for UserName {
    type Err = InvalidUserName;

    fn from_str(s: &str) -> Result<UserName, InvalidUserName> {
        if !s.is_empty()
            && s.len() <= 30
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            Ok(UserName(s.into()))
        } else {
            Err(InvalidUserName)
        }
    }
}

#[derive(Debug)]
pub struct UserId(String);

impl From<UserName> for UserId {
    fn from(UserName(mut name): UserName) -> UserId {
        name.make_ascii_lowercase();
        UserId(name)
    }
}

impl UserId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
