#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum Speed {
    Ultrabullet,
    Bullet,
    Blitz,
    Rapid,
    Classical,
    Correspondence,
}

#[derive(Debug, Default)]
pub struct BySpeed<T> {
    pub ultrabullet: T,
    pub bullet: T,
    pub blitz: T,
    pub rapid: T,
    pub classical: T,
    pub correspondence: T,
}

impl<T> BySpeed<T> {
    pub fn by_speed(&self, speed: Speed) -> &T {
        match speed {
            Speed::Ultrabullet => &self.ultrabullet,
            Speed::Bullet => &self.bullet,
            Speed::Blitz => &self.blitz,
            Speed::Rapid => &self.rapid,
            Speed::Classical => &self.classical,
            Speed::Correspondence => &self.correspondence,
        }
    }

    pub fn by_speed_mut(&mut self, speed: Speed) -> &mut T {
        match speed {
            Speed::Ultrabullet => &mut self.ultrabullet,
            Speed::Bullet => &mut self.bullet,
            Speed::Blitz => &mut self.blitz,
            Speed::Rapid => &mut self.rapid,
            Speed::Classical => &mut self.classical,
            Speed::Correspondence => &mut self.correspondence,
        }
    }

    pub fn as_ref(&self) -> BySpeed<&T> {
        BySpeed {
            ultrabullet: &self.ultrabullet,
            bullet: &self.bullet,
            blitz: &self.blitz,
            rapid: &self.rapid,
            classical: &self.classical,
            correspondence: &self.correspondence,
        }
    }

    pub fn try_map<U, E, F>(self, mut f: F) -> Result<BySpeed<U>, E>
    where
        F: FnMut(Speed, T) -> Result<U, E>,
    {
        Ok(BySpeed {
            ultrabullet: f(Speed::Ultrabullet, self.ultrabullet)?,
            bullet: f(Speed::Bullet, self.bullet)?,
            blitz: f(Speed::Blitz, self.blitz)?,
            rapid: f(Speed::Rapid, self.rapid)?,
            classical: f(Speed::Classical, self.classical)?,
            correspondence: f(Speed::Correspondence, self.correspondence)?,
        })
    }
}
