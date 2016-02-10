package lila.openingexplorer

import ornicar.scalalib.Validation
import scala.concurrent.ExecutionContext.Implicits.global
import scala.concurrent.Future
import scalaz.Validation.FlatMap._

import orestes.bloomfilter.BloomFilter

import chess.format.Forsyth
import chess.format.pgn.{ Parser, Reader, ParsedPgn, San }
import chess.variant.Variant
import chess.{ Hash, PositionHash, Replay }

final class Importer(
    masterDb: MasterDatabase,
    lichessDb: LichessDatabase,
    pgnDb: PgnDatabase,
    filter: BloomFilter[String]) extends Validation with scalaz.syntax.ToValidationOps {

  private val lichessSeparator = "\n\n\n"

  def lichess(variant: Variant, text: String): (Unit, Int) = Time {
    val pgns = text.split(lichessSeparator)
    pgns flatMap { origPgn =>
      val pgn = if (variant.exotic) s"[Variant ${variant.name}]\n$origPgn" else origPgn
      process(pgn, fastPgn = true, Config.explorer.lichess(variant).maxPlies) match {
        case scalaz.Success(processed) => Some(processed)
        case scalaz.Failure(errors) =>
          play.api.Logger("importer").warn(errors.list mkString ", ")
          None
      }
    } foreach {
      case Processed(_, replay, gameRef) =>
        if (filter.contains(gameRef.gameId))
          play.api.Logger("importer").warn(s"probable duplicate: ${gameRef.gameId}, err = ${filter.getFalsePositiveProbability}")
        else {
          Future(filter.add(gameRef.gameId))
          lichessDb.merge(variant, gameRef, collectHashes(replay, LichessDatabase.hash))
        }
    }
  }

  private val masterInitBoard = chess.Board.init(chess.variant.Standard)
  private val masterMinRating = 2200

  def master(pgn: String): (Valid[Unit], Int) = Time {
    process(pgn, fastPgn = false, Config.explorer.master.maxPlies) flatMap {
      case Processed(parsed, replay, gameRef) =>
        if (filter.contains(gameRef.gameId))
          s"probable duplicate: ${gameRef.gameId}, err = ${filter.getFalsePositiveProbability}".failureNel
        else if ((Forsyth >> replay.setup.situation) != Forsyth.initial)
          s"Invalid initial position ${Forsyth >> replay.setup.situation}".failureNel
        else if (gameRef.averageRating < masterMinRating)
          s"Average rating ${gameRef.averageRating} < $masterMinRating".failureNel
        else scalaz.Success {
          Future(filter.add(gameRef.gameId))
          masterDb.merge(gameRef, collectHashes(replay, MasterDatabase.hash))
          pgnDb.store(gameRef.gameId, parsed, replay)
        }
    }
  }

  private case class Processed(parsed: ParsedPgn, replay: Replay, gameRef: GameRef)

  private def process(pgn: String, fastPgn: Boolean, maxPlies: Int): Valid[Processed] = for {
    parsed <- if (fastPgn) parseFastPgn(pgn) else Parser.full(pgn)
    replay <- makeReplay(parsed, maxPlies)
    gameRef <- GameRef.fromPgn(parsed)
  } yield Processed(parsed, replay, gameRef)

  private def parseFastPgn(pgn: String): Valid[ParsedPgn] = pgn.split("\n\n") match {
    case Array(tagStr, moveStr) => for {
      tags ← Parser.TagParser(tagStr)
      moves <- Parser.moves(moveStr, Parser.getVariantFromTags(tags))
    } yield ParsedPgn(tags, moves)
    case _ => s"Invalid fast pgn $pgn".failureNel
  }

  private def Time[A](f: => A): (A, Int) = {
    val start = System.currentTimeMillis
    val res = f
    res -> (System.currentTimeMillis - start).toInt
  }

  private def makeReplay(parsed: ParsedPgn, maxPlies: Int): Valid[Replay] = {
    def truncateMoves(moves: List[San]) = moves take maxPlies
    Reader.fullWithSans(parsed, truncateMoves _)
  }

  private def collectHashes(replay: Replay, hash: Hash) = Util.distinctHashes({
    replay.setup.situation :: replay.moves.map(_.fold(_.situationAfter, _.situationAfter))
  }.map(hash.apply))
}
