package lila.openingexplorer

import akka.actor.CoordinatedShutdown
import fm.last.commons.kyoto.factory.{ Compressor, PageComparator }
import javax.inject.{ Inject, Singleton }

import chess.format.{ FEN, Forsyth }
import chess.format.pgn.{ Move, ParsedPgn, Pgn, Tag, TagType, Tags, Turn }
import chess.Replay

@Singleton
final class PgnDatabase @Inject() (
    config: Config,
    shutdown: CoordinatedShutdown
)(implicit ec: scala.concurrent.ExecutionContext) {

  private val db = Util.wrapLog(
    "Loading PGN database...",
    "PGN database loaded!"
  ) {
    Kyoto
      .builder(config.explorer.pgn.kyoto)
      .compressor(Compressor.LZMA)
      .pageComparator(PageComparator.LEXICAL)
      .buildAndOpen
  }

  private val relevantTags: Set[TagType] =
    Tag.tagTypes.toSet diff Set(Tag.ECO, Tag.Opening, Tag.Variant)

  def get(gameId: String): Option[String] = Option(db.get(gameId))

  def store(gameId: String, parsed: ParsedPgn, replay: Replay): Boolean = {

    val tags = parsed.tags.value.filter { tag => relevantTags contains tag.name }
    val fenSituation = tags find (_.name == Tag.FEN) flatMap { case Tag(_, fen) =>
      Forsyth <<< FEN(fen)
    }
    val pgnMoves = replay.chronoMoves
      .foldLeft(replay.setup) { case (game, moveOrDrop) =>
        moveOrDrop.fold(game.apply, game.applyDrop)
      }
      .pgnMoves
    val moves       = if (fenSituation.exists(_.situation.color.black)) ".." +: pgnMoves else pgnMoves
    val initialTurn = fenSituation.map(_.fullMoveNumber) getOrElse 1
    val pgn         = Pgn(Tags(tags), turns(moves, initialTurn))

    db.putIfAbsent(gameId, pgn.toString)
  }

  private def turns(moves: Vector[String], from: Int): List[Turn] =
    (moves grouped 2).zipWithIndex.toList map { case (moves, index) =>
      Turn(
        number = index + from,
        white = moves.headOption filter (".." !=) map { Move(_) },
        black = moves lift 1 map { Move(_) }
      )
    } filterNot (_.isEmpty)

  def delete(gameId: String) = db.remove(gameId)

  def count = db.recordCount()

  shutdown.addTask(CoordinatedShutdown.PhaseServiceStop, "close master db") { () =>
    scala.concurrent.Future {
      db.close()
      akka.Done
    }
  }
}
