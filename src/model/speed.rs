use std::{array, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Eq, PartialEq, Copy, Clone, Deserialize, Serialize, Ord, PartialOrd)]
#[serde(rename_all = "camelCase")]
pub enum Speed {
    UltraBullet,
    Bullet,
    Blitz,
    Rapid,
    Classical,
    Correspondence,
}

impl Speed {
    pub const ALL: [Speed; 6] = [
        Speed::UltraBullet,
        Speed::Bullet,
        Speed::Blitz,
        Speed::Rapid,
        Speed::Classical,
        Speed::Correspondence,
    ];
}

impl FromStr for Speed {
    type Err = InvalidSpeed;

    fn from_str(s: &str) -> Result<Speed, InvalidSpeed> {
        Ok(match s {
            "ultraBullet" => Speed::UltraBullet,
            "bullet" => Speed::Bullet,
            "blitz" => Speed::Blitz,
            "rapid" => Speed::Rapid,
            "classical" => Speed::Classical,
            "correspondence" => Speed::Correspondence,
            _ => return Err(InvalidSpeed),
        })
    }
}

#[derive(Error, Debug)]
#[error("invalid speed")]
pub struct InvalidSpeed;

#[derive(Debug, Default)]
pub struct BySpeed<T> {
    pub ultra_bullet: T,
    pub bullet: T,
    pub blitz: T,
    pub rapid: T,
    pub classical: T,
    pub correspondence: T,
}

impl<T> BySpeed<T> {
    pub fn by_speed_mut(&mut self, speed: Speed) -> &mut T {
        match speed {
            Speed::UltraBullet => &mut self.ultra_bullet,
            Speed::Bullet => &mut self.bullet,
            Speed::Blitz => &mut self.blitz,
            Speed::Rapid => &mut self.rapid,
            Speed::Classical => &mut self.classical,
            Speed::Correspondence => &mut self.correspondence,
        }
    }

    pub fn as_ref(&self) -> BySpeed<&T> {
        BySpeed {
            ultra_bullet: &self.ultra_bullet,
            bullet: &self.bullet,
            blitz: &self.blitz,
            rapid: &self.rapid,
            classical: &self.classical,
            correspondence: &self.correspondence,
        }
    }

    pub fn zip_speed(self) -> BySpeed<(Speed, T)> {
        BySpeed {
            ultra_bullet: (Speed::UltraBullet, self.ultra_bullet),
            bullet: (Speed::Bullet, self.bullet),
            blitz: (Speed::Blitz, self.blitz),
            rapid: (Speed::Rapid, self.rapid),
            classical: (Speed::Classical, self.classical),
            correspondence: (Speed::Correspondence, self.correspondence),
        }
    }
}

impl<T> IntoIterator for BySpeed<T> {
    type Item = T;
    type IntoIter = array::IntoIter<T, 6>;

    fn into_iter(self) -> Self::IntoIter {
        [
            self.ultra_bullet,
            self.bullet,
            self.blitz,
            self.rapid,
            self.classical,
            self.correspondence,
        ]
        .into_iter()
    }
}
