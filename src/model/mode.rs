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
}
