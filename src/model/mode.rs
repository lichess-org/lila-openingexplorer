#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Mode {
    Rated,
    Casual,
}

impl Mode {
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
