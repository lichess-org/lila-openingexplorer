package lila.openingexplorer

import ornicar.scalalib.Validation
import scala.concurrent.ExecutionContext.Implicits.global
import scala.concurrent.Future
import scalaz.Validation.FlatMap._

import chess.format.Forsyth
import chess.format.pgn.{ Parser, Reader, ParsedPgn, San }
import chess.variant.Variant
import chess.{ Hash, PositionHash, Replay }

final class Importer(
    masterDb: MasterDatabase,
    lichessDb: LichessDatabase,
    pgnDb: PgnDatabase,
    gameInfoDb: GameInfoDatabase) extends Validation with scalaz.syntax.ToValidationOps {

  private val lichessSeparator = "\n\n\n"

  def lichess(text: String): (Unit, Int) = Time {
    val pgns = text.split(lichessSeparator)
    pgns flatMap { pgn =>
      processLichess(pgn) match {
        case scalaz.Success(processed) => Some(processed)
        case scalaz.Failure(errors) =>
          play.api.Logger("importer").warn(errors.list mkString ", ")
          None
      }
    } foreach {
      case Processed(parsed, replay, gameRef) =>
        GameInfo parse parsed.tags match {
          case _ if gameInfoDb.contains(gameRef.gameId) =>
            play.api.Logger("importer").warn("skip dup ${gameRef.gameId}")
          case None =>
            play.api.Logger("importer").warn(s"Can't produce GameInfo for game ${gameRef.gameId}")
          case Some(info) =>
            val variant = replay.setup.board.variant
            lichessDb.merge(variant, gameRef, collectHashes(replay, LichessDatabase.hash))
            gameInfoDb.store(gameRef.gameId, info)
        }
    }
  }

  private val masterInitBoard = chess.Board.init(chess.variant.Standard)
  private val masterMinRating = 2200

  def master(pgn: String): (Valid[Unit], Int) = Time {
    processMaster(pgn, Config.explorer.master.maxPlies) flatMap {
      case Processed(parsed, replay, gameRef) =>
        if ((Forsyth >> replay.setup.situation) != Forsyth.initial)
          s"Invalid initial position ${Forsyth >> replay.setup.situation}".failureNel
        else if (gameRef.averageRating < masterMinRating)
          s"Average rating ${gameRef.averageRating} < $masterMinRating".failureNel
        else scalaz.Success {
          masterDb.merge(gameRef, collectHashes(replay, MasterDatabase.hash))
          pgnDb.store(gameRef.gameId, parsed, replay)
        }
    }
  }

  private case class Processed(parsed: ParsedPgn, replay: Replay, gameRef: GameRef)

  private def processMaster(pgn: String, maxPlies: Int): Valid[Processed] = for {
    parsed <- Parser.full(pgn)
    replay <- makeReplay(parsed, maxPlies)
    gameRef <- GameRef.fromPgn(parsed)
  } yield Processed(parsed, replay, gameRef)

  private def processLichess(pgn: String): Valid[Processed] = for {
    parsed <- parseFastPgn(pgn)
    variant = Parser.getVariantFromTags(parsed.tags)
    replay <- makeReplay(parsed, Config.explorer.lichess(variant).maxPlies)
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
