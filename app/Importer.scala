package lila.openingexplorer

import ornicar.scalalib.Validation
import scalaz.Validation.FlatMap._

import chess.format.pgn.{ Parser, ParsedPgn, San }
import chess.variant.Variant
import chess.{ Hash, PositionHash }

final class Importer(
    masterDb: MasterDatabase,
    lichessDb: LichessDatabase) extends Validation {

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
      case (parsed, gameRef) =>
        lichessDb.mergeAll(variant, collectHashes(parsed, LichessDatabase.hash), gameRef)
    }
  }

  def master(pgn: String): (Valid[Unit], Int) = Time {
    process(pgn) map {
      case (parsed, gameRef) =>
        masterDb.mergeAll(collectHashes(parsed, MasterDatabase.hash), gameRef)
    }
  }

  private def process(pgn: String) = for {
    parsed <- Parser.full(pgn)
    gameRef <- GameRef.fromPgn(parsed)
  } yield parsed -> gameRef

  private def Time[A](f: => A): (A, Int) = {
    val start = System.currentTimeMillis
    val res = f
    res -> (System.currentTimeMillis - start).toInt
  }

  private def collectHashes(parsed: ParsedPgn, hash: Hash): Set[PositionHash] = {
    def truncateMoves(moves: List[San]) = moves take 40
    chess.format.pgn.Reader.fullWithSans(parsed, truncateMoves _) match {
      case scalaz.Success(replay) => {
        // the starting position
        List(hash(replay.moves.last.fold(_.situationBefore, _.situationBefore))) ++
          // all others
          replay.moves.map(_.fold(_.situationAfter, _.situationAfter)).map(hash(_))
      }.toSet
      case scalaz.Failure(errors) =>
        play.api.Logger("importer").warn(errors.list mkString ", ")
        Set.empty
    }
  }
}
