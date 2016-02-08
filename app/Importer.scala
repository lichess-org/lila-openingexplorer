package lila.openingexplorer

import ornicar.scalalib.Validation
import scalaz.Validation.FlatMap._

import chess.format.pgn.{ Parser, ParsedPgn, San }
import chess.PositionHash
import chess.variant.Variant

final class Importer(db: MasterDatabase) extends Validation {

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
      case (parsed, gameRef) => db.mergeAll(collectHashes(parsed), gameRef)
    }
  }

  def master(pgn: String): (Valid[Unit], Int) = Time {
    process(pgn) map {
      case (parsed, gameRef) => db.mergeAll(collectHashes(parsed), gameRef)
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

  private def collectHashes(parsed: chess.format.pgn.ParsedPgn): Set[PositionHash] = {
    def truncateMoves(moves: List[San]) = moves take 40
    chess.format.pgn.Reader.fullWithSans(parsed, truncateMoves _) match {
      case scalaz.Success(replay) => {
        // the starting position
        List(db.hash(replay.moves.last.fold(_.situationBefore, _.situationBefore))) ++
          // all others
          replay.moves.map(_.fold(_.situationAfter, _.situationAfter)).map(db.hash(_))
      }.toSet
      case scalaz.Failure(errors) =>
        play.api.Logger("importer").warn(errors.list mkString ", ")
        Set.empty
    }
  }
}
