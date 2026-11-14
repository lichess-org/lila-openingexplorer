use std::{fmt, str::FromStr};

use arbitrary::Arbitrary;
use bitflags::bitflags;
use btoi::btoi;
use bytes::{Buf, BufMut};
use shakmaty::ByColor;
use thiserror::Error;

use crate::packed::{pack_int, unpack_int};

#[derive(Arbitrary, Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rating {
    offset: i16,
}

impl Rating {
    const BASE: i32 = 1500;

    /// Packed in place of the offset of a missing rating. Outside the range
    /// of offsets.
    const MISSING: i64 = i16::MAX as i64 + 1;

    pub fn offset(self) -> i16 {
        self.offset
    }

    pub fn from_offset(offset: i16) -> Self {
        Rating { offset }
    }

    pub fn pack_fixed<B: BufMut>(&self, buf: &mut B) {
        buf.put_i16(self.offset);
    }

    pub fn unpack_fixed<B: Buf>(buf: &mut B) -> Self {
        Rating::from_offset(buf.get_i16())
    }

    pub fn pack_optional<B: BufMut>(rating: Option<Rating>, buf: &mut B) {
        pack_int(
            buf,
            rating.map_or(Rating::MISSING, |rating| i64::from(rating.offset)),
        );
    }

    pub fn unpack_optional<B: Buf>(buf: &mut B) -> Option<Rating> {
        match unpack_int(buf) {
            Rating::MISSING => None,
            offset => Some(Rating::from_offset(
                i16::try_from(offset).expect("valid rating"),
            )),
        }
    }

    pub fn from_ascii(bytes: &[u8]) -> Result<Self, InvalidRating> {
        btoi::<i32>(bytes)
            .map_err(|_| InvalidRating)
            .and_then(Rating::try_from)
    }
}

#[derive(Debug, Error)]
#[error("invalid rating")]
pub struct InvalidRating;

impl FromStr for Rating {
    type Err = InvalidRating;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_ascii(s.as_bytes())
    }
}

impl From<Rating> for i32 {
    fn from(rating: Rating) -> Self {
        i32::from(rating.offset) + Rating::BASE
    }
}

impl TryFrom<i32> for Rating {
    type Error = InvalidRating;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Ok(Rating {
            offset: value
                .saturating_sub(Rating::BASE)
                .try_into()
                .map_err(|_| InvalidRating)?,
        })
    }
}

impl fmt::Debug for Rating {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rating({})", i32::from(*self))
    }
}

#[derive(Arbitrary, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RatingGroup {
    Low,
    Group1000,
    Group1200,
    Group1400,
    Group1600,
    Group1800,
    Group2000,
    Group2200,
    Group2500,
    Group2800,
    Group3200,
}

impl RatingGroup {
    pub const ALL: [RatingGroup; 11] = [
        RatingGroup::Low,
        RatingGroup::Group1000,
        RatingGroup::Group1200,
        RatingGroup::Group1400,
        RatingGroup::Group1600,
        RatingGroup::Group1800,
        RatingGroup::Group2000,
        RatingGroup::Group2200,
        RatingGroup::Group2500,
        RatingGroup::Group2800,
        RatingGroup::Group3200,
    ];

    pub fn bits(self) -> u8 {
        match self {
            RatingGroup::Low => 0,
            RatingGroup::Group1000 => 1,
            RatingGroup::Group1200 => 2,
            RatingGroup::Group1400 => 3,
            RatingGroup::Group1600 => 4,
            RatingGroup::Group1800 => 5,
            RatingGroup::Group2000 => 6,
            RatingGroup::Group2200 => 7,
            RatingGroup::Group2500 => 8,
            RatingGroup::Group2800 => 9,
            RatingGroup::Group3200 => 10,
        }
    }

    pub fn from_bits(bits: u8) -> Option<RatingGroup> {
        Some(match bits {
            0 => RatingGroup::Low,
            1 => RatingGroup::Group1000,
            2 => RatingGroup::Group1200,
            3 => RatingGroup::Group1400,
            4 => RatingGroup::Group1600,
            5 => RatingGroup::Group1800,
            6 => RatingGroup::Group2000,
            7 => RatingGroup::Group2200,
            8 => RatingGroup::Group2500,
            9 => RatingGroup::Group2800,
            10 => RatingGroup::Group3200,
            _ => return None,
        })
    }

    /// By the average rating of both players, so that all positions of a
    /// game are in the same group.
    pub fn select_game(ratings: ByColor<Rating>) -> RatingGroup {
        RatingGroup::select_i32(i32::midpoint(ratings.white.into(), ratings.black.into()))
    }

    pub fn select(rating: Rating) -> RatingGroup {
        RatingGroup::select_i32(rating.into())
    }

    fn select_i32(rating: i32) -> RatingGroup {
        if rating < 1000 {
            RatingGroup::Low
        } else if rating < 1200 {
            RatingGroup::Group1000
        } else if rating < 1400 {
            RatingGroup::Group1200
        } else if rating < 1600 {
            RatingGroup::Group1400
        } else if rating < 1800 {
            RatingGroup::Group1600
        } else if rating < 2000 {
            RatingGroup::Group1800
        } else if rating < 2200 {
            RatingGroup::Group2000
        } else if rating < 2500 {
            RatingGroup::Group2200
        } else if rating < 2800 {
            RatingGroup::Group2500
        } else if rating < 3200 {
            RatingGroup::Group2800
        } else {
            RatingGroup::Group3200
        }
    }

    pub(crate) fn typical_rating(self) -> Rating {
        Rating::try_from(match self {
            RatingGroup::Low => 884,
            RatingGroup::Group1000 => 1111,
            RatingGroup::Group1200 => 1307,
            RatingGroup::Group1400 => 1504,
            RatingGroup::Group1600 => 1701,
            RatingGroup::Group1800 => 1896,
            RatingGroup::Group2000 => 2089,
            RatingGroup::Group2200 => 2310,
            RatingGroup::Group2500 => 2600,
            RatingGroup::Group2800 => 2925,
            RatingGroup::Group3200 => 3269,
        })
        .expect("typical rating")
    }
}

bitflags! {
    /// A set of rating groups.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct RatingGroups: u16 {
        const LOW = 1 << 0;
        const GROUP_1000 = 1 << 1;
        const GROUP_1200 = 1 << 2;
        const GROUP_1400 = 1 << 3;
        const GROUP_1600 = 1 << 4;
        const GROUP_1800 = 1 << 5;
        const GROUP_2000 = 1 << 6;
        const GROUP_2200 = 1 << 7;
        const GROUP_2500 = 1 << 8;
        const GROUP_2800 = 1 << 9;
        const GROUP_3200 = 1 << 10;
    }
}

impl From<RatingGroup> for RatingGroups {
    fn from(group: RatingGroup) -> RatingGroups {
        match group {
            RatingGroup::Low => RatingGroups::LOW,
            RatingGroup::Group1000 => RatingGroups::GROUP_1000,
            RatingGroup::Group1200 => RatingGroups::GROUP_1200,
            RatingGroup::Group1400 => RatingGroups::GROUP_1400,
            RatingGroup::Group1600 => RatingGroups::GROUP_1600,
            RatingGroup::Group1800 => RatingGroups::GROUP_1800,
            RatingGroup::Group2000 => RatingGroups::GROUP_2000,
            RatingGroup::Group2200 => RatingGroups::GROUP_2200,
            RatingGroup::Group2500 => RatingGroups::GROUP_2500,
            RatingGroup::Group2800 => RatingGroups::GROUP_2800,
            RatingGroup::Group3200 => RatingGroups::GROUP_3200,
        }
    }
}

impl FromIterator<RatingGroup> for RatingGroups {
    fn from_iter<I: IntoIterator<Item = RatingGroup>>(iter: I) -> RatingGroups {
        iter.into_iter().map(RatingGroups::from).collect()
    }
}

/// Parses the group of a rating, like `RatingGroup::select()`. Lower
/// bounds of groups, like `1600`, select that group.
impl FromStr for RatingGroup {
    type Err = InvalidRating;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(RatingGroup::select)
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_i32_roundtrip() {
        arbtest(|u| {
            let rating: Rating = u.arbitrary()?;

            assert_eq!(Rating::try_from(i32::from(rating)).unwrap(), rating);
            Ok(())
        });
    }

    #[test]
    fn test_pack_optional_roundtrip() {
        arbtest(|u| {
            let rating: Option<Rating> = u.arbitrary()?;

            let mut buf = Vec::new();
            Rating::pack_optional(rating, &mut buf);
            assert_eq!(Rating::unpack_optional(&mut &buf[..]), rating);
            Ok(())
        });
    }

    #[test]
    fn test_select_game() {
        let group = |white: i32, black: i32| {
            RatingGroup::select_game(ByColor {
                white: Rating::try_from(white).unwrap(),
                black: Rating::try_from(black).unwrap(),
            })
        };
        assert_eq!(group(2600, 2300), RatingGroup::Group2200);
        assert_eq!(group(2300, 2600), RatingGroup::Group2200);
        assert_eq!(group(2700, 2300), RatingGroup::Group2500);
        assert_eq!(group(2499, 2500), RatingGroup::Group2200);
        assert_eq!(group(2500, 2501), RatingGroup::Group2500);
        assert_eq!(group(999, 1000), RatingGroup::Low);
        assert_eq!(group(600, 3400), RatingGroup::Group2000);
    }

    #[test]
    fn test_pack_fixed_roundtrip() {
        arbtest(|u| {
            let rating: Rating = u.arbitrary()?;

            let mut buf = [0; 2];
            rating.pack_fixed(&mut &mut buf[..]);
            assert_eq!(Rating::unpack_fixed(&mut &buf[..]), rating);
            Ok(())
        });
    }
}
