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
    masters_to_parquet::id::{IdInput, push_uci},
    packed::{
        EventInfo, GameId, GamePlayer, LaxDate, LichessGame, PackedMoves, Rating, RatingMode,
    },
};

pub struct MastersVisitor;

#[derive(Default)]
pub struct ParsedMastersGame {
    event: Option<String>,
    site: Option<String>,
    date: Option<String>,
    round: Option<String>,
    players: ByColor<Option<String>>,
    elos: ByColor<Option<i32>>,
    result: Option<Outcome>,
    fen: Option<Fen>,
    moves: Vec<San>,
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn utf8_tag(name: &str, value: RawTag<'_>) -> io::Result<String> {
    value
        .decode_utf8()
        .map(|s| s.into_owned())
        .map_err(|err| invalid(format!("invalid {name} tag: {err}")))
}

fn elo_tag(name: &str, value: RawTag<'_>) -> io::Result<i32> {
    btoi::btoi(value.as_bytes()).map_err(|_| {
        invalid(format!(
            "invalid {name} tag {:?}",
            value.decode_utf8_lossy()
        ))
    })
}

impl ParsedMastersGame {
    fn visit_tag(&mut self, name: &[u8], value: RawTag<'_>) -> io::Result<()> {
        match name {
            b"Event" => self.event = Some(utf8_tag("Event", value)?),
            b"Site" => self.site = Some(utf8_tag("Site", value)?),
            b"Date" => self.date = Some(utf8_tag("Date", value)?),
            b"Round" => self.round = Some(utf8_tag("Round", value)?),
            b"White" => self.players.white = Some(utf8_tag("White", value)?),
            b"Black" => self.players.black = Some(utf8_tag("Black", value)?),
            b"WhiteElo" => self.elos.white = Some(elo_tag("WhiteElo", value)?),
            b"BlackElo" => self.elos.black = Some(elo_tag("BlackElo", value)?),
            b"Result" => {
                self.result = Some(Outcome::from_ascii(value.as_bytes()).map_err(|err| {
                    invalid(format!(
                        "invalid Result tag {:?}: {err}",
                        value.decode_utf8_lossy()
                    ))
                })?);
            }
            b"FEN" => {
                self.fen = Some(Fen::from_ascii(value.as_bytes()).map_err(|err| {
                    invalid(format!(
                        "invalid FEN tag {:?}: {err}",
                        value.decode_utf8_lossy()
                    ))
                })?);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn plies(&self) -> usize {
        self.moves.len()
    }

    fn context(&self) -> String {
        format!(
            "{} - {}, {}, {}",
            self.players.white.as_deref().unwrap_or("?"),
            self.players.black.as_deref().unwrap_or("?"),
            self.event.as_deref().unwrap_or("?"),
            self.date.as_deref().unwrap_or("?"),
        )
    }

    pub fn into_game(self) -> io::Result<(GameId, LichessGame)> {
        let context = self.context();
        let required = |name: &str, value: Option<String>| {
            value.ok_or_else(|| invalid(format!("missing {name} tag @ {context}")))
        };

        let event = required("Event", self.event)?;
        let site = required("Site", self.site)?;
        let date = required("Date", self.date)?;
        let round = required("Round", self.round)?;
        let white = required("White", self.players.white)?;
        let black = required("Black", self.players.black)?;
        let white_elo = self
            .elos
            .white
            .ok_or_else(|| invalid(format!("missing WhiteElo tag @ {context}")))?;
        let black_elo = self
            .elos
            .black
            .ok_or_else(|| invalid(format!("missing BlackElo tag @ {context}")))?;
        let outcome = self
            .result
            .and_then(Outcome::known)
            .ok_or_else(|| invalid(format!("unknown result @ {context}")))?;

        let lax_date = LaxDate::from_ascii(date.as_bytes())
            .map_err(|err| invalid(format!("invalid Date tag {date:?}: {err} @ {context}")))?;

        let (mut pos, setup) = match self.fen {
            None => (VariantPosition::new(Variant::Chess), None),
            Some(fen) => {
                let pos =
                    VariantPosition::from_setup(Variant::Chess, fen.into(), CastlingMode::Chess960)
                        .map_err(|err| invalid(format!("invalid setup: {err} @ {context}")))?;
                let setup = PackedSetup::pack_variant_normalized(
                    &pos.to_setup(EnPassantMode::Legal),
                    Variant::Chess,
                )
                .map_err(|err| invalid(format!("unsupported setup: {err} @ {context}")))?;
                (pos, Some(setup))
            }
        };

        let mut encoder = PackedMoves::encoder_with_capacity(self.moves.len());
        let mut uci_moves = String::with_capacity(self.moves.len() * 5);
        for san in &self.moves {
            let m = san
                .to_move(&pos)
                .map_err(|err| invalid(format!("illegal san {san}: {err} @ {context}")))?;
            encoder.encode_next(&pos, m);
            push_uci(&mut uci_moves, m);
            pos.play_unchecked(m);
        }

        let game_id = IdInput {
            event: &event,
            site: &site,
            date: &date,
            round: &round,
            players: ByColor {
                white: (&white, white_elo),
                black: (&black, black_elo),
            },
            outcome,
            moves: &uci_moves,
        }
        .game_id();

        let rating = |elo: i32| {
            Rating::try_from(elo)
                .map_err(|err| invalid(format!("invalid elo {elo}: {err} @ {context}")))
        };

        Ok((
            game_id,
            LichessGame {
                players: ByColor {
                    white: GamePlayer {
                        title: None,
                        name: white,
                        rating: Some(rating(white_elo)?),
                    },
                    black: GamePlayer {
                        title: None,
                        name: black,
                        rating: Some(rating(black_elo)?),
                    },
                },
                date: lax_date,
                outcome,
                speed: None,
                rating_mode: RatingMode::Rated,
                setup,
                event: Some(EventInfo { event, site, round }),
                moves: encoder.finish(),
            },
        ))
    }
}

impl Visitor for MastersVisitor {
    type Tags = ParsedMastersGame;
    type Movetext = ParsedMastersGame;
    type Output = io::Result<ParsedMastersGame>;

    fn begin_tags(&mut self) -> ControlFlow<Self::Output, Self::Tags> {
        ControlFlow::Continue(ParsedMastersGame::default())
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
        movetext.moves.push(san_plus.san);
        ControlFlow::Continue(())
    }

    fn end_game(&mut self, movetext: Self::Movetext) -> Self::Output {
        Ok(movetext)
    }
}
