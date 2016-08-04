package lila.openingexplorer

import ornicar.scalalib.Validation
import scala.concurrent.ExecutionContext.Implicits.global
import scala.concurrent.Future
import scalaz.Validation.FlatMap._

import chess.format.Forsyth
import chess.format.pgn.{ Parser, Reader, ParsedPgn, InitialPosition, San }
import chess.variant.Variant
import chess.{ Hash, PositionHash, Replay }

final class Importer(
    masterDb: MasterDatabase,
    lichessDb: LichessDatabase,
    pgnDb: PgnDatabase,
    gameInfoDb: GameInfoDatabase) extends Validation with scalaz.syntax.ToValidationOps {

  private val lichessSeparator = "\n\n\n"

  private val logger = play.api.Logger("importer")

  private var nbImported = 0

  def lichess(text: String): (Unit, Int) = Time {
    val pgns = text.split(lichessSeparator)
    val processed = pgns flatMap { pgn =>
      processLichess(pgn) match {
        case scalaz.Success(processed) => Some(processed)
        case scalaz.Failure(errors) =>
          logger.warn(errors.list mkString ", ")
          None
      }
    }
    if (processed.exists { p => gameInfoDb.contains(p.gameRef.gameId) })
      logger.info(s"found a dup, skipping batch")
    else {
      processed foreach {
        case Processed(parsed, replay, gameRef) =>
          GameInfo parse parsed.tags match {
            case None => logger.warn(s"Can't produce GameInfo for game ${gameRef.gameId}")
            case Some(info) =>
              val variant = replay.setup.board.variant
              try {
                gameInfoDb.store(gameRef.gameId, info)

                replay.chronoMoves.take(Config.explorer.lichess(variant).maxPlies).foreach {
                  move =>
                    lichessDb.merge(variant, gameRef, move)
                }
              } catch {
                case e: Exception => logger.error(s"Can't merge game ${gameRef.gameId}: ${e.getMessage}")
              }
          }
      }
      val nb = processed.size
      nbImported = nbImported + nb
      logger.info(s"Imported $nb lichess games; total $nbImported")
    }
  }

  private val masterInitBoard = chess.Board.init(chess.variant.Standard)
  private val masterMinRating = 2200

  def master(pgn: String): (Valid[Unit], Int) = Time {
    processMaster(pgn) flatMap {
      case Processed(parsed, replay, gameRef) =>
        if ((Forsyth >> replay.setup.situation) != Forsyth.initial)
          s"Invalid initial position ${Forsyth >> replay.setup.situation}".failureNel
        else if (gameRef.averageRating < masterMinRating)
          s"Average rating ${gameRef.averageRating} < $masterMinRating".failureNel
        else scalaz.Success {
          pgnDb.store(gameRef.gameId, parsed, replay)

          replay.chronoMoves.take(Config.explorer.master.maxPlies).foreach {
            move =>
              masterDb.merge(gameRef, move)
          }
        }
    }
  }

  private case class Processed(parsed: ParsedPgn, replay: Replay, gameRef: GameRef)

  private def processMaster(pgn: String): Valid[Processed] = for {
    parsed <- Parser.full(pgn)
    replay <- Reader.fullWithSans(parsed, identity[List[San]] _)
    gameRef <- GameRef.fromPgn(parsed)
  } yield Processed(parsed, replay, gameRef)

  private def processLichess(pgn: String): Valid[Processed] = for {
    parsed <- parseFastPgn(pgn)
    variant = Parser.getVariantFromTags(parsed.tags)
    replay <- Reader.fullWithSans(parsed, (moves: List[San]) => moves.take(Config.explorer.lichess(variant).maxPlies))
    gameRef <- GameRef.fromPgn(parsed)
  } yield Processed(parsed, replay, gameRef)

  private def parseFastPgn(pgn: String): Valid[ParsedPgn] = pgn.split("\n\n") match {
    case Array(tagStr, moveStr) => for {
      tags ← Parser.TagParser(tagStr)
      moves <- Parser.moves(moveStr, Parser.getVariantFromTags(tags))
    } yield ParsedPgn(InitialPosition(List.empty), tags, moves)
    case _ => s"Invalid fast pgn $pgn".failureNel
  }

  private def Time[A](f: => A): (A, Int) = {
    val start = System.currentTimeMillis
    val res = f
    res -> (System.currentTimeMillis - start).toInt
  }
}
