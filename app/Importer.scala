package lila.openingexplorer

import ornicar.scalalib.Validation
import scalaz.Validation.FlatMap._

import chess.format.pgn.{ Parser, Reader, ParsedPgn, San }
import chess.variant.Variant
import chess.{ Hash, PositionHash, Replay }

final class Importer(
    masterDb: MasterDatabase,
    lichessDb: LichessDatabase,
    pgnDb: PgnDatabase) extends Validation {

  private val lichessSeparator = "\n\n"

  def lichess(variant: Variant, text: String): (Unit, Int) = Time {
    text.split(lichessSeparator).toList flatMap { pgn =>
      process(pgn) match {
        case scalaz.Success(processed) => Some(processed)
        case scalaz.Failure(errors) =>
          play.api.Logger("importer").warn(errors.list mkString ", ")
          None
      }
    } foreach {
      case (replay, gameRef) =>
        lichessDb.merge(variant, gameRef, collectHashes(replay, LichessDatabase.hash))
    }
  }

  def master(pgn: String): (Valid[Unit], Int) = Time {
    process(pgn) map {
      case (replay, gameRef) =>
        masterDb.merge(gameRef, collectHashes(replay, MasterDatabase.hash))
        pgnDb.store(replay)
    }
  }

  private def process(pgn: String) = for {
    parsed <- Parser.full(pgn)
    replay <- makeReplay(parsed)
    gameRef <- GameRef.fromPgn(parsed)
  } yield replay -> gameRef

  private def Time[A](f: => A): (A, Int) = {
    val start = System.currentTimeMillis
    val res = f
    res -> (System.currentTimeMillis - start).toInt
  }

  private def makeReplay(parsed: ParsedPgn): Valid[Replay] = {
    def truncateMoves(moves: List[San]) = moves take 40
    Reader.fullWithSans(parsed, truncateMoves _)
  }

  private def collectHashes(replay: Replay, hash: Hash): Set[PositionHash] = {
    List(hash(replay.moves.last.fold(_.situationBefore, _.situationBefore))) ++
      // all others
      replay.moves.map(_.fold(_.situationAfter, _.situationAfter)).map(hash(_))
  }.toSet
}
