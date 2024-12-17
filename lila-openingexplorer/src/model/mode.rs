use std::{array, str::FromStr};

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize)]
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

#[derive(Error, Debug)]
#[error("invalid mode")]
pub struct InvalidMode;

#[derive(Default, Debug)]
pub struct ByMode<T> {
    pub rated: T,
    pub casual: T,
}

impl<T> ByMode<T> {
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

    pub fn zip_mode(self) -> ByMode<(Mode, T)> {
        ByMode {
            rated: (Mode::Rated, self.rated),
            casual: (Mode::Casual, self.casual),
        }
    }
}

impl<T> IntoIterator for ByMode<T> {
    type Item = T;
    type IntoIter = array::IntoIter<T, 2>;

    fn into_iter(self) -> Self::IntoIter {
        [self.rated, self.casual].into_iter()
    }
}
