package lila.openingexplorer

import ornicar.scalalib.Validation
import scala.collection.parallel.CollectionConverters._
import scalaz.Validation.FlatMap._
import javax.inject.{ Inject, Singleton }

import chess.format.Forsyth
import chess.format.pgn.{ InitialPosition, ParsedPgn, Parser, Reader, Sans }
import chess.{ Replay }

@Singleton
final class Importer @Inject() (
    config: Config,
    masterDb: MasterDatabase,
    lichessDb: LichessDatabase,
    pgnDb: PgnDatabase,
    gameInfoDb: GameInfoDatabase
) extends Validation
    with scalaz.syntax.ToValidationOps {

  private val lichessSeparator = "\n\n\n"

  private val logger = play.api.Logger("importer")

  private var nbImported = 0

  def lichess(text: String): (Unit, Int) = Time {
    val pgns = text.split(lichessSeparator)
    val processed = pgns.par flatMap { pgn =>
      processLichess(pgn) match {
        case scalaz.Success(processed) => Some(processed)
        case scalaz.Failure(errors) =>
          logger.warn(errors.list.toList mkString ", ")
          None
      }
    }
    processed foreach {
      case Processed(parsed, replay, gameRef) =>
        GameInfo parse parsed.tags match {
          case None => logger.warn(s"Can't produce GameInfo for game ${gameRef.gameId}")
          case Some(info) =>
            val variant = replay.setup.board.variant
            try {
              if (gameInfoDb.store(gameRef.gameId, info)) {
                replay.chronoMoves.take(config.explorer.lichess(variant).maxPlies).foreach { move =>
                  lichessDb.merge(variant, gameRef, move)
                }
              } else {
                logger.warn(s"Duplicate lichess game ${gameRef.gameId}")
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

  private val masterMinRating = 2200

  def master(pgn: String): (Valid[Unit], Int) = Time {
    processMaster(pgn) flatMap {
      case Processed(parsed, replay, gameRef) => {
        val moves = replay.chronoMoves.take(config.explorer.master.maxPlies)
        if ((Forsyth >> replay.setup.situation) != Forsyth.initial)
          s"Invalid initial position: ${Forsyth >> replay.setup.situation}".failureNel
        else if (gameRef.averageRating < masterMinRating)
          s"Skipping average rating: ${gameRef.averageRating} < $masterMinRating".failureNel
        else if (moves.isEmpty)
          s"No moves in game".failureNel
        else if (masterDb.exists(moves.last.fold(_.situationBefore, _.situationBefore)))
          s"Likely duplicate: ${parsed.tags("White").getOrElse("?")} vs. ${parsed
            .tags("Black")
            .getOrElse("?")} (${parsed.tags("Date").getOrElse("????.??.??")})".failureNel
        else if (pgnDb.store(gameRef.gameId, parsed, replay)) {
          scalaz.Success {
            moves.foreach { move => masterDb.merge(gameRef, move) }
          }
        } else {
          s"Duplicate master game id: ${gameRef.gameId}".failureNel
        }
      }
    }
  }

  def deleteMaster(gameId: String) = {
    pgnDb.get(gameId) map { pgn =>
      processMaster(pgn) flatMap {
        case Processed(parsed, replay, newGameRef) =>
          scalaz.Success {
            val gameRef = newGameRef.copy(gameId = gameId)
            replay.chronoMoves.take(config.explorer.master.maxPlies).foreach { move =>
              masterDb.subtract(gameRef, move)
            }
            pgnDb.delete(gameRef.gameId)
          }
      }
      true
    } getOrElse false
  }

  private case class Processed(parsed: ParsedPgn, replay: Replay, gameRef: GameRef)

  private def processMaster(pgn: String): Valid[Processed] =
    for {
      parsed  <- Parser.full(pgn)
      replay  <- Reader.fullWithSans(parsed, identity[Sans] _).valid
      gameRef <- GameRef.fromMasterPgn(parsed)
    } yield Processed(parsed, replay, gameRef)

  private def processLichess(pgn: String): Valid[Processed] =
    for {
      parsed  <- parseFastPgn(pgn)
      variant <- parsed.tags.variant toValid "Invalid variant"
      replay <- Reader
        .fullWithSans(
          parsed,
          (moves: Sans) =>
            Sans {
              moves.value.take(config.explorer.lichess(variant).maxPlies)
            }
        )
        .valid
      gameRef <- GameRef.fromLichessPgn(parsed)
    } yield Processed(parsed, replay, gameRef)

  private def parseFastPgn(pgn: String): Valid[ParsedPgn] = pgn.split("\n\n") match {
    case Array(tagStr, moveStr) =>
      for {
        tags    <- Parser.TagParser(tagStr)
        variant <- tags.variant toValid "Invalid variant"
        moves   <- Parser.moves(moveStr, variant)
      } yield ParsedPgn(InitialPosition(List.empty), tags, moves)
    case _ => s"Invalid fast pgn $pgn".failureNel
  }

  private def Time[A](f: => A): (A, Int) = {
    val start = System.currentTimeMillis
    val res   = f
    res -> (System.currentTimeMillis - start).toInt
  }
}
