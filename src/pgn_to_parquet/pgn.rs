use std::{io, ops::ControlFlow};

use pgn_reader::{RawTag, Skip, Visitor};
use shakmaty::{
    ByColor, CastlingMode, EnPassantMode, Outcome, Position,
    fen::Fen,
    packed::PackedSetup,
    san::{San, SanPlus},
    variant::{Variant, VariantPosition},
};

use crate::{
    lila_variant::LilaVariant,
    packed::{
        BOT_TITLE, GameId, GamePlayer, LaxDate, LichessGame, PackedMoves, Rating, RatingMode, Speed,
    },
};

/// Parses games, keeping only the first `max_ply` moves.
///
/// Moves after `max_ply` are not validated.
pub struct GameVisitor {
    pub max_ply: usize,
}

#[derive(Default)]
struct ParsedPlayer {
    name: Option<String>,
    title: Option<String>,
    rating: Option<Rating>,
}

#[derive(Default)]
pub struct ParsedGame {
    game_id: Option<GameId>,
    players: ByColor<ParsedPlayer>,
    speed: Option<Speed>,
    date: Option<LaxDate>,
    variant: Option<LilaVariant>,
    fen: Option<Fen>,
    moves: Vec<San>,
    outcome: Option<Outcome>,
}

impl ParsedGame {
    fn utf8_tag(&self, name: &str, value: RawTag<'_>) -> io::Result<String> {
        String::from_utf8(value.as_bytes().to_owned()).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid {} tag: {} @ {:?}", name, err, self.game_id),
            )
        })
    }

    fn rating_tag(&self, name: &str, value: RawTag<'_>) -> io::Result<Rating> {
        Rating::from_ascii(value.as_bytes()).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid {} tag {:?}: {} @ {:?}",
                    name,
                    value.decode_utf8_lossy(),
                    err,
                    self.game_id
                ),
            )
        })
    }

    fn visit_tag(&mut self, name: &[u8], value: RawTag<'_>) -> io::Result<()> {
        match name {
            b"Site" => {
                // https://lichess.org/{id}, since 2026-10 also
                // https://lichess.org/game/{id}.
                self.game_id = value
                    .as_bytes()
                    .strip_prefix(b"https://lichess.org/")
                    .map(|path| path.strip_prefix(b"game/").unwrap_or(path))
                    .map(GameId::from_ascii)
                    .transpose()
                    .map_err(|err| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "invalid game id in Site tag {:?}: {}",
                                value.decode_utf8_lossy(),
                                err
                            ),
                        )
                    })?;
            }
            b"White" => self.players.white.name = Some(self.utf8_tag("White", value)?),
            b"Black" => self.players.black.name = Some(self.utf8_tag("Black", value)?),
            b"WhiteElo" if value.as_bytes() != b"?" => {
                self.players.white.rating = Some(self.rating_tag("WhiteElo", value)?);
            }
            b"BlackElo" if value.as_bytes() != b"?" => {
                self.players.black.rating = Some(self.rating_tag("BlackElo", value)?);
            }
            b"WhiteTitle" if !value.as_bytes().is_empty() => {
                self.players.white.title = Some(self.utf8_tag("WhiteTitle", value)?);
            }
            b"BlackTitle" if !value.as_bytes().is_empty() => {
                self.players.black.title = Some(self.utf8_tag("BlackTitle", value)?);
            }
            b"UTCDate" => {
                self.date = Some(LaxDate::from_ascii(value.as_bytes()).map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "invalid Date tag {:?}: {} @ {:?}",
                            value.decode_utf8_lossy(),
                            err,
                            self.game_id
                        ),
                    )
                })?);
            }
            b"TimeControl" => {
                self.speed = Some(Speed::from_time_control_tag(value.as_bytes()).map_err(
                    |err| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "invalid TimeControl tag {:?}: {} @ {:?}",
                                value.decode_utf8_lossy(),
                                err,
                                self.game_id
                            ),
                        )
                    },
                )?);
            }
            b"Variant" => {
                self.variant =
                    Some(LilaVariant::from_pgn_tag(value.as_bytes()).map_err(|err| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "invalid Variant tag {:?}: {} @ {:?}",
                                value.decode_utf8_lossy(),
                                err,
                                self.game_id
                            ),
                        )
                    })?);
            }
            b"FEN" => {
                self.fen = Some(Fen::from_ascii(value.as_bytes()).map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "invalid FEN tag {:?}: {} @ {:?}",
                            value.decode_utf8_lossy(),
                            err,
                            self.game_id
                        ),
                    )
                })?);
            }
            _ => {}
        }
        Ok(())
    }

    fn visit_san(&mut self, san: San, max_ply: usize) {
        if self.moves.len() < max_ply {
            self.moves.push(san);
        }
    }

    fn visit_outcome(&mut self, outcome: Outcome) {
        self.outcome = Some(outcome);
    }

    pub fn game_id(&self) -> Option<GameId> {
        self.game_id
    }

    pub fn plies(&self) -> usize {
        self.moves.len()
    }

    pub fn lila_variant(&self) -> LilaVariant {
        self.variant.unwrap_or_default()
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome.unwrap_or(Outcome::Unknown)
    }

    pub fn has_bot(&self) -> bool {
        self.players
            .iter()
            .any(|player| player.title.as_deref() == Some(BOT_TITLE))
    }

    pub fn has_ratings(&self) -> bool {
        self.players.iter().all(|player| player.rating.is_some())
    }

    fn initial_position(&self) -> io::Result<(VariantPosition, Option<PackedSetup>)> {
        let variant = self.lila_variant().variant();

        let (pos, need_setup) = match self.fen.clone() {
            None => (VariantPosition::new(variant), variant != Variant::Chess),
            Some(fen) => (
                VariantPosition::from_setup(variant, fen.into(), CastlingMode::Chess960).map_err(
                    |err| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("invalid setup: {} @ {:?}", err, self.game_id),
                        )
                    },
                )?,
                true,
            ),
        };

        let setup = need_setup
            .then(|| {
                // Legal, but can still be unrepresentable, e.g., more than 15
                // pieces of a kind in a crazyhouse pocket.
                PackedSetup::pack_variant_normalized(&pos.to_setup(EnPassantMode::Legal), variant)
                    .map_err(|err| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unsupported setup: {} @ {:?}", err, self.game_id),
                        )
                    })
            })
            .transpose()?;

        Ok((pos, setup))
    }

    pub fn into_game(self) -> io::Result<(GameId, LichessGame)> {
        let game_id = self
            .game_id
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing game id"))?;

        let outcome = self.outcome().known().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown outcome @ {:?}", game_id),
            )
        })?;

        let (mut pos, setup) = self.initial_position()?;

        let mut encoder = PackedMoves::encoder_with_capacity(self.moves.len());

        for san in &self.moves {
            let m = san.to_move(&pos).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("illegal san {}: {} @ {:?}", san, err, game_id),
                )
            })?;

            encoder.encode_next(&pos, m);
            pos.play_unchecked(m);
        }

        let moves = encoder.finish();

        let game = LichessGame {
            players: self.players.zip_color().try_map(|(color, player)| {
                Ok::<_, io::Error>(GamePlayer {
                    title: player.title,
                    name: player.name.ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("missing {} name @ {:?}", color, game_id),
                        )
                    })?,
                    rating: player.rating,
                })
            })?,
            date: self.date.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("missing date @ {:?}", game_id),
                )
            })?,
            outcome,
            speed: self.speed,
            rating_mode: RatingMode::Rated, // always rated for monthly export
            setup,
            event: None,
            moves,
        };

        Ok((game_id, game))
    }
}

impl Visitor for GameVisitor {
    type Tags = ParsedGame;
    type Movetext = ParsedGame;
    type Output = io::Result<ParsedGame>;

    fn begin_tags(&mut self) -> ControlFlow<Self::Output, Self::Tags> {
        ControlFlow::Continue(ParsedGame::default())
    }

    fn tag(
        &mut self,
        tags: &mut Self::Tags,
        name: &[u8],
        value: RawTag<'_>,
    ) -> ControlFlow<Self::Output> {
        match tags.visit_tag(name, value) {
            Ok(()) => ControlFlow::Continue(()),
            Err(err) => ControlFlow::Break(Err(err)),
        }
    }

    fn begin_movetext(&mut self, tags: Self::Tags) -> ControlFlow<Self::Output, Self::Movetext> {
        ControlFlow::Continue(tags)
    }

    fn begin_variation(
        &mut self,
        _movetext: &mut Self::Movetext,
    ) -> ControlFlow<Self::Output, Skip> {
        ControlFlow::Continue(Skip(true))
    }

    fn san(
        &mut self,
        movetext: &mut Self::Movetext,
        san_plus: SanPlus,
    ) -> ControlFlow<Self::Output> {
        movetext.visit_san(san_plus.san, self.max_ply);
        ControlFlow::Continue(())
    }

    fn outcome(
        &mut self,
        movetext: &mut Self::Movetext,
        outcome: Outcome,
    ) -> ControlFlow<Self::Output> {
        movetext.visit_outcome(outcome);
        ControlFlow::Continue(())
    }

    fn end_game(&mut self, movetext: Self::Movetext) -> Self::Output {
        Ok(movetext)
    }
}

#[cfg(test)]
mod tests {
    use pgn_reader::Reader;

    use super::*;

    fn game_ids(pgn: &str) -> Vec<String> {
        Reader::new(pgn.as_bytes())
            .read_games(&mut GameVisitor {
                max_ply: usize::MAX,
            })
            .map(|game| {
                game.expect("io")
                    .expect("valid game")
                    .game_id()
                    .expect("game id")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn test_has_bot() {
        let pgn = "[Site \"https://lichess.org/aaaaaaaa\"]\n[WhiteTitle \"BOT\"]\n\n1. e4 e5 0-1\n\n\
                   [Site \"https://lichess.org/bbbbbbbb\"]\n[BlackTitle \"BOT\"]\n\n1. e4 e5 0-1\n\n\
                   [Site \"https://lichess.org/cccccccc\"]\n[WhiteTitle \"GM\"]\n[BlackTitle \"\"]\n\n1. e4 e5 0-1\n\n\
                   [Site \"https://lichess.org/dddddddd\"]\n[White \"BOT\"]\n\n1. e4 e5 0-1\n";
        let bots: Vec<bool> = Reader::new(pgn.as_bytes())
            .read_games(&mut GameVisitor {
                max_ply: usize::MAX,
            })
            .map(|game| game.expect("io").expect("valid game").has_bot())
            .collect();
        assert_eq!(bots, [true, true, false, false]);
    }

    #[test]
    fn test_has_ratings() {
        let pgn = "[Site \"https://lichess.org/aaaaaaaa\"]\n[WhiteElo \"1500\"]\n[BlackElo \"1600\"]\n\n1. e4 e5 0-1\n\n\
                   [Site \"https://lichess.org/bbbbbbbb\"]\n[WhiteElo \"?\"]\n[BlackElo \"1600\"]\n\n1. e4 e5 0-1\n\n\
                   [Site \"https://lichess.org/cccccccc\"]\n[WhiteElo \"1500\"]\n\n1. e4 e5 0-1\n";
        let ratings: Vec<bool> = Reader::new(pgn.as_bytes())
            .read_games(&mut GameVisitor {
                max_ply: usize::MAX,
            })
            .map(|game| game.expect("io").expect("valid game").has_ratings())
            .collect();
        assert_eq!(ratings, [true, false, false]);
    }

    #[test]
    fn test_site_formats() {
        let pgn = "[Site \"https://lichess.org/bcRJNu4b\"]\n\n1. e4 e5 0-1\n\n\
                   [Site \"https://lichess.org/game/Q8ku5wgM\"]\n\n1. d4 d5 1/2-1/2\n";
        assert_eq!(game_ids(pgn), ["bcRJNu4b", "Q8ku5wgM"]);
    }
}
