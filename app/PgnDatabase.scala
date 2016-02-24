package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.factory.{ KyotoDbBuilder, Mode, Compressor, PageComparator }
import fm.last.commons.kyoto.{ KyotoDb, WritableVisitor }

import chess.format.Forsyth
import chess.format.pgn.{ ParsedPgn, Pgn, Tag, TagType, Dumper, Turn, Move }
import chess.Replay

final class PgnDatabase extends MasterDatabasePacker {

  private val db = Util.wrapLog(
    "Loading PGN database...",
    "PGN database loaded!") {
      val config = Config.explorer.pgn
      val dbFile = new File(config.kyoto.file)
      dbFile.createNewFile

      new KyotoDbBuilder(dbFile)
        .modes(Mode.CREATE, Mode.READ_WRITE)
        .buckets(config.kyoto.buckets)
        .memoryMapSize(config.kyoto.memoryMapSize)
        .defragUnitSize(config.kyoto.defragUnitSize)
        .compressor(Compressor.LZMA)
        .pageComparator(PageComparator.LEXICAL)
        .buildAndOpen
    }

  private val relevantTags: Set[TagType] =
    Tag.tagTypes.toSet diff Set(Tag.ECO, Tag.Opening, Tag.Variant)

  def get(gameId: String): Option[String] = Option(db.get(gameId))

  def store(gameId: String, parsed: ParsedPgn, replay: Replay) = {

    val tags = parsed.tags.filter { tag =>
      relevantTags contains tag.name
    }
    val fenSituation = tags find (_.name == Tag.FEN) flatMap {
      case Tag(_, fen) => Forsyth <<< fen
    }
    val pgnMoves = replay.chronoMoves.foldLeft(replay.setup) {
      case (game, moveOrDrop) => moveOrDrop.fold(game.apply, game.applyDrop)
    }.pgnMoves
    val moves = if (fenSituation.exists(_.situation.color.black)) ".." :: pgnMoves else pgnMoves
    val initialTurn = fenSituation.map(_.fullMoveNumber) getOrElse 1
    val pgn = Pgn(tags, turns(moves, initialTurn))

    db.set(gameId, pgn.toString)
  }

  private def turns(moves: List[String], from: Int): List[Turn] =
    (moves grouped 2).zipWithIndex.toList map {
      case (moves, index) => Turn(
        number = index + from,
        white = moves.headOption filter (".." !=) map { Move(_) },
        black = moves lift 1 map { Move(_) })
    } filterNot (_.isEmpty)

  def delete(gameId: String) = db.remove(gameId)

  def count = db.recordCount()

  def close = {
    db.close()
  }
}
