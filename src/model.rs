use std::convert::TryFrom;
use std::fmt::{self, Write};
use std::str::FromStr;

#[derive(Debug)]
struct InvalidGameId;

#[derive(Debug, Clone, Eq, PartialEq)]
struct GameId(u64);

impl TryFrom<u64> for GameId {
    type Error = InvalidGameId;

    fn try_from(n: u64) -> Result<Self, Self::Error> {
        if n < 62u64.pow(8) {
            Ok(GameId(n))
        } else {
            Err(InvalidGameId)
        }
    }
}

impl FromStr for GameId {
    type Err = InvalidGameId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if dbg!(s).len() != 8 {
            return Err(InvalidGameId);
        }

        let mut n = 0;
        let mut digit = 1;
        for c in s.bytes() {
            n += match c {
                b'0'..=b'9' => c - b'0',
                b'A'..=b'Z' => c - b'A' + 10,
                b'a'..=b'z' => c - b'a' + 10 + 26,
                _ => return Err(InvalidGameId),
            } as u64 * digit;
            digit *= 62;
        }

        Ok(GameId(n))
    }
}

impl fmt::Display for GameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut n = self.0;
        for _ in 0..8 {
            let rem = n % 62;
            f.write_char(char::from(if rem >= 10 + 26 {
                (rem - (10 + 26)) as u8 + b'a'
            } else if rem >= 10 {
                (rem - 10) as u8 + b'A'
            } else {
                rem as u8 + b'0'
            }))?;
            n /= 62;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use quickcheck::{Arbitrary, Gen, quickcheck};
    use super::*;

    impl Arbitrary for GameId {
        fn arbitrary(g: &mut Gen) -> Self {
            GameId(u64::arbitrary(g) % 62u64.pow(8))
        }
    }

    quickcheck! {
        fn game_id_roundtrip(game_id: GameId) -> bool {
            GameId::from_str(&game_id.to_string()).unwrap() == game_id
        }
    }
}
