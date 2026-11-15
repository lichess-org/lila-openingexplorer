use shakmaty::{
    Bitboard, Move, Position, Rank, Role, Square, attacks,
    variant::{VariantPosition, WithPosition},
};
use thiserror::Error;

use crate::packed::bits::{BitReader, BitWriter};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackedMoves(pub Vec<u8>);

impl PackedMoves {
    pub fn encoder() -> PackedMovesEncoder {
        PackedMovesEncoder::default()
    }

    pub fn encoder_with_capacity(plies: usize) -> PackedMovesEncoder {
        PackedMovesEncoder {
            writer: BitWriter::new(Vec::with_capacity(plies + 1)),
        }
    }

    pub fn encode(pos: &mut VariantPosition, moves: impl IntoIterator<Item = Move>) -> Self {
        let moves = moves.into_iter();
        let mut encoder = PackedMoves::encoder_with_capacity(moves.size_hint().0);
        for m in moves {
            encoder.encode_next(pos, m);
            pos.play_unchecked(m);
        }
        encoder.finish()
    }

    pub fn decode(&self) -> Result<PackedMovesDecoder<'_>, InvalidPackedMoves> {
        PackedMovesDecoder::new(&self.0)
    }

    pub fn truncated(
        bytes: &[u8],
        mut pos: VariantPosition,
        max_ply: usize,
    ) -> Result<PackedMoves, InvalidPackedMoves> {
        let mut decoder = PackedMovesDecoder::new(bytes)?;
        let mut encoder = PackedMoves::encoder_with_capacity(max_ply);
        for _ in 0..max_ply {
            let Some(m) = decoder.decode_next(&pos)? else {
                break;
            };
            encoder.encode_next(&pos, m);
            pos.play_unchecked(m);
        }
        Ok(encoder.finish())
    }
}

pub struct PackedMovesEncoder {
    writer: BitWriter<Vec<u8>>,
}

impl Default for PackedMovesEncoder {
    fn default() -> PackedMovesEncoder {
        PackedMovesEncoder {
            writer: BitWriter::new(Vec::new()),
        }
    }
}

impl PackedMovesEncoder {
    #[inline]
    pub fn encode_next(&mut self, pos: &VariantPosition, m: Move) {
        struct EncodeMove<'a> {
            writer: &'a mut BitWriter<Vec<u8>>,
            m: Move,
        }

        impl WithPosition for EncodeMove<'_> {
            type Output = ();

            #[inline]
            fn call<P: Position>(self, pos: &P) {
                encode_move(self.writer, pos, self.m);
            }
        }

        pos.with_position(EncodeMove {
            writer: &mut self.writer,
            m,
        });
    }

    pub fn finish(self) -> PackedMoves {
        PackedMoves(self.writer.finish_terminated())
    }
}

pub struct PackedMovesDecoder<'a> {
    reader: BitReader<'a>,
}

impl PackedMovesDecoder<'_> {
    pub fn new(bytes: &[u8]) -> Result<PackedMovesDecoder<'_>, InvalidPackedMoves> {
        let reader = BitReader::terminated(bytes).ok_or(InvalidPackedMoves)?;
        Ok(PackedMovesDecoder { reader })
    }

    #[inline]
    pub fn decode_next<P: Position>(
        &mut self,
        pos: &P,
    ) -> Result<Option<Move>, InvalidPackedMoves> {
        Ok(if self.reader.has_remaining() {
            Some(decode_move(&mut self.reader, pos)?)
        } else {
            None
        })
    }
}

#[derive(Debug, Error)]
#[error("invalid packed moves")]
pub struct InvalidPackedMoves;

#[inline]
fn bits_for(slots: usize) -> u32 {
    // Same as slots.next_power_of_two().ilog2()
    usize::BITS - slots.saturating_sub(1).leading_zeros()
}

#[inline]
fn role_index(role: Role) -> u64 {
    u64::from(role) - 1
}

#[inline]
fn square_index(sq: Square, mask: Bitboard) -> u64 {
    let below = mask & Bitboard((1 << u32::from(sq)) - 1);
    below.count() as u64
}

#[inline]
fn nth_square(mask: Bitboard, n: usize) -> Option<Square> {
    #[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
    {
        if fast_pdep() {
            if n >= 64 {
                return None;
            }

            // SAFETY: Part of bmi2 target feature.
            return Bitboard(unsafe { std::arch::x86_64::_pdep_u64(1 << n, mask.0) }).first();
        }
    }
    mask.nth(n)
}

#[cfg(all(target_arch = "x86_64", target_feature = "bmi2"))]
#[inline]
fn fast_pdep() -> bool {
    static FAST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FAST.get_or_init(|| {
        // Everything with bmi2 except AMD family 17h.
        let (vendor, info) = (std::arch::x86_64::__cpuid(0), std::arch::x86_64::__cpuid(1));
        let amd = (vendor.ebx, vendor.edx, vendor.ecx) == (0x6874_7541, 0x6974_6e65, 0x444d_4163);
        let family = ((info.eax >> 8) & 0xf) + ((info.eax >> 20) & 0xff);
        !(amd && family == 0x17)
    })
}

#[inline]
fn pseudo_legal_targets<P: Position>(pos: &P, role: Role, from: Square) -> Bitboard {
    match role {
        Role::Pawn => {
            let forward = pos.turn().fold_wb(8, -8);
            let one_step = Bitboard::from_square(from).shift(forward) & !pos.board().occupied();
            let two_step = one_step.shift(forward)
                & !pos.board().occupied()
                & pos.turn().fold_wb(Bitboard::SOUTH, Bitboard::NORTH);
            let ep = pos
                .maybe_ep_square()
                .map_or(Bitboard::EMPTY, Bitboard::from_square);
            let captures = attacks::pawn_attacks(pos.turn(), from) & (pos.them() | ep);
            one_step | two_step | captures
        }
        Role::Knight => attacks::knight_attacks(from) & !pos.us(),
        Role::Bishop => attacks::bishop_attacks(from, pos.board().occupied()) & !pos.us(),
        Role::Rook => attacks::rook_attacks(from, pos.board().occupied()) & !pos.us(),
        Role::Queen => attacks::queen_attacks(from, pos.board().occupied()) & !pos.us(),
        Role::King => {
            let mut targets = attacks::king_attacks(from) & !pos.us();
            if from.rank() == pos.turn().backrank() {
                targets.add(pos.castles().castling_rights() & pos.turn().backrank());
            }
            targets
        }
    }
}

fn encode_move<P: Position>(writer: &mut BitWriter<Vec<u8>>, pos: &P, m: Move) {
    let piece_bits = bits_for(pos.us().count() + usize::from(pos.pockets().is_some()));
    if let Some(from) = m.from() {
        let targets = pseudo_legal_targets(pos, m.role(), from);
        let target_bits = bits_for(targets.count());
        writer.write(square_index(from, pos.us()), piece_bits);
        writer.write(square_index(m.to(), targets), target_bits);
        if let Some(promotion) = m.promotion() {
            writer.write(role_index(promotion), bits_for(6));
        } else if piece_bits == 0 && target_bits == 0 {
            writer.write(0, 1); // Force at least one bit per move
        }
    } else {
        // Drop move
        writer.write(pos.us().count() as u64, piece_bits);
        writer.write(role_index(m.role()), bits_for(6));
        writer.write(u64::from(m.to()), bits_for(64));
    }
}

#[inline]
fn decode_move<P: Position>(reader: &mut BitReader, pos: &P) -> Result<Move, InvalidPackedMoves> {
    let piece_bits = bits_for(pos.us().count() + usize::from(pos.pockets().is_some()));
    let piece_index = reader.read(piece_bits) as usize;
    if piece_index < pos.us().count() {
        let from = nth_square(pos.us(), piece_index).ok_or(InvalidPackedMoves)?;

        let role = pos.board().role_at(from).ok_or(InvalidPackedMoves)?;

        let targets = pseudo_legal_targets(pos, role, from);
        let target_bits = bits_for(targets.count());
        let to =
            nth_square(targets, reader.read(target_bits) as usize).ok_or(InvalidPackedMoves)?;

        // Deliberately fused with &, to make a single rarely taken branch.
        let promotion =
            if (role == Role::Pawn) & (to.rank() == pos.turn().relative_rank(Rank::Eighth)) {
                Some(Role::try_from(reader.read(bits_for(6)) + 1).map_err(|_| InvalidPackedMoves)?)
            } else {
                if piece_bits == 0 && target_bits == 0 && reader.read(1) != 0 {
                    return Err(InvalidPackedMoves);
                }
                None
            };

        let castle = (role == Role::King) & pos.us().contains(to);
        let en_passant = (role == Role::Pawn)
            & (from.file() != to.file())
            & !pos.board().occupied().contains(to);
        Ok(if castle {
            Move::Castle {
                king: from,
                rook: to,
            }
        } else if en_passant {
            Move::EnPassant { from, to }
        } else {
            Move::Normal {
                role,
                from,
                capture: pos.board().role_at(to),
                to,
                promotion,
            }
        })
    } else if piece_index == pos.us().count() && pos.pockets().is_some() {
        Ok(Move::Put {
            role: Role::try_from(reader.read(bits_for(6)) + 1).map_err(|_| InvalidPackedMoves)?,
            to: Square::new(reader.read(bits_for(64)) as u32),
        })
    } else {
        Err(InvalidPackedMoves)
    }
}

#[cfg(test)]
mod tests {
    use arbitrary::Arbitrary;
    use arbtest::arbtest;
    use shakmaty::{CastlingMode, fen::Fen, san::San};

    use super::*;

    struct Game {
        pos: VariantPosition,
        moves: Vec<Move>,
    }

    impl Arbitrary<'_> for Game {
        fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
            let len: usize = u.arbitrary_len::<usize>()?;
            let mut game = Game {
                pos: u.arbitrary()?,
                moves: Vec::with_capacity(len),
            };

            let mut pos = game.pos.clone();
            for _ in 0..len {
                let legals = pos.legal_moves();
                let Ok(&m) = u.choose(&legals) else { break };
                pos.play_unchecked(m);
                game.moves.push(m);
            }

            Ok(game)
        }
    }

    /// Truncated moves are the first moves, packed like those alone (so
    /// unchanged if there are no more).
    #[test]
    fn test_truncated() {
        arbtest(|u| {
            let game: Game = u.arbitrary()?;
            let max_ply = u.int_in_range(0..=game.moves.len() + 2)?;
            let packed = PackedMoves::encode(&mut game.pos.clone(), game.moves.iter().copied());
            let expected = PackedMoves::encode(
                &mut game.pos.clone(),
                game.moves.iter().copied().take(max_ply),
            );
            assert_eq!(
                PackedMoves::truncated(&packed.0, game.pos.clone(), max_ply).unwrap(),
                expected
            );
            Ok(())
        });
    }

    #[test]
    fn test_moves_roundtrip() {
        arbtest(|u| {
            let game: Game = u.arbitrary()?;

            let packed_moves =
                PackedMoves::encode(&mut game.pos.clone(), game.moves.iter().copied());

            let mut decoder = packed_moves.decode().unwrap();
            let mut pos = game.pos.clone();
            for m in game.moves {
                let decoded = decoder.decode_next(&pos).unwrap().unwrap();
                assert_eq!(decoded, m);
                pos.play_unchecked(decoded);
            }
            assert!(decoder.decode_next(&pos).unwrap().is_none());

            Ok(())
        });
    }

    #[test]
    fn test_forced_moves() {
        // shoopi, www.chess.com 6/11/2013, Mate In 8
        let mut pos = VariantPosition::Chess(
            "rk1KB3/b1q1QQQR/bnqQ1q2/1Rq1Q1q1/2q1rQ1q/6B1/6QQ/8 w - - 0 1"
                .parse::<Fen>()
                .unwrap()
                .into_position(CastlingMode::Chess960)
                .unwrap(),
        );

        let moves: Vec<Move> = [
            "Qxc7+", "Qxc7+", "Qxc7+", "Qxc7+", "Qxc7+", "Qxc7+", "Bxc7+", "Kb7+", "Bb8+",
        ]
        .iter()
        .scan(pos.clone(), |pos, san| {
            let m = san.parse::<San>().unwrap().to_move(pos).unwrap();
            pos.play_unchecked(m);
            Some(m)
        })
        .collect();

        let packed_moves = PackedMoves::encode(&mut pos.clone(), moves.iter().copied());

        let mut decoder = packed_moves.decode().unwrap();
        for m in moves {
            let decoded = decoder.decode_next(&pos).unwrap().unwrap();
            assert_eq!(decoded, m);
            pos.play_unchecked(decoded);
        }

        assert!(decoder.decode_next(&pos).unwrap().is_none());
    }

    #[test]
    fn test_forced_en_passant() {
        let mut pos = VariantPosition::Antichess(
            "8/3p4/4p3/4P3/8/8/8/8 b - - 0 1"
                .parse::<Fen>()
                .unwrap()
                .into_position(CastlingMode::Chess960)
                .unwrap(),
        );

        let moves: Vec<Move> = ["d5", "exd6"]
            .iter()
            .scan(pos.clone(), |pos, san| {
                let m = san.parse::<San>().unwrap().to_move(pos).unwrap();
                pos.play_unchecked(m);
                Some(m)
            })
            .collect();

        assert!(moves[1].is_en_passant());

        let packed_moves = PackedMoves::encode(&mut pos.clone(), moves.iter().copied());

        let mut decoder = packed_moves.decode().unwrap();
        for m in moves {
            let decoded = decoder.decode_next(&pos).unwrap().unwrap();
            assert_eq!(decoded, m);
            pos.play_unchecked(decoded);
        }

        assert!(decoder.decode_next(&pos).unwrap().is_none());
    }
}
