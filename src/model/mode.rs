use std::{error::Error as StdError, fmt, ops::AddAssign, str::FromStr};

use serde::Deserialize;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Rated,
    Casual,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Rated, Mode::Casual];

    pub fn from_rated(rated: bool) -> Mode {
        if rated {
            Mode::Rated
        } else {
            Mode::Casual
        }
    }

    pub fn is_rated(self) -> bool {
        self == Mode::Rated
    }
}

impl FromStr for Mode {
    type Err = InvalidMode;

    fn from_str(s: &str) -> Result<Mode, InvalidMode> {
        Ok(match s {
            "rated" => Mode::Rated,
            "casual" => Mode::Casual,
            _ => return Err(InvalidMode),
        })
    }
}

#[derive(Debug)]
pub struct InvalidMode;

impl fmt::Display for InvalidMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid mode")
    }
}

impl StdError for InvalidMode {}

#[derive(Default, Debug)]
pub struct ByMode<T> {
    pub rated: T,
    pub casual: T,
}

impl<T> ByMode<T> {
    pub fn by_mode(&self, mode: Mode) -> &T {
        match mode {
            Mode::Rated => &self.rated,
            Mode::Casual => &self.casual,
        }
    }

    pub fn by_mode_mut(&mut self, mode: Mode) -> &mut T {
        match mode {
            Mode::Rated => &mut self.rated,
            Mode::Casual => &mut self.casual,
        }
    }

    pub fn as_ref(&self) -> ByMode<&T> {
        ByMode {
            rated: &self.rated,
            casual: &self.casual,
        }
    }

    pub fn try_map<U, E, F>(self, mut f: F) -> Result<ByMode<U>, E>
    where
        F: FnMut(Mode, T) -> Result<U, E>,
    {
        Ok(ByMode {
            rated: f(Mode::Rated, self.rated)?,
            casual: f(Mode::Casual, self.casual)?,
        })
    }
}

impl<T: AddAssign> AddAssign for ByMode<T> {
    fn add_assign(&mut self, rhs: ByMode<T>) {
        self.rated += rhs.rated;
        self.casual += rhs.casual;
    }
}
