package lila.openingexplorer

import chess.format.Uci
import java.io.{ OutputStream, InputStream }

case class SubEntry(
    moves: Map[Either[Uci.Move, Uci.Drop], MoveStats],
    gameRefs: List[GameRef]
) extends PackHelper {

  lazy val totalWhite = moves.values.map(_.white).sum
  lazy val totalDraws = moves.values.map(_.draws).sum
  lazy val totalBlack = moves.values.map(_.black).sum

  def totalGames = totalWhite + totalDraws + totalBlack

  def isEmpty = totalGames == 0

  def totalAverageRatingSum = moves.values.map(_.averageRatingSum).sum

  def averageRating: Int =
    if (totalGames == 0) 0 else (totalAverageRatingSum / totalGames).toInt

  def withGameRef(game: GameRef, move: Either[Uci.Move, Uci.Drop]) =
    new SubEntry(
      moves + (move -> moves.getOrElse(move, MoveStats.empty).withGameRef(game)),
      game :: gameRefs
    )

  def withExistingGameRef(game: GameRef) = copy(gameRefs = game :: gameRefs)

  def withoutExistingGameRef(game: GameRef, move: Either[Uci.Move, Uci.Drop]) = {
    val stats =
      moves.get(move)
        .map(_.withoutExistingGameRef(game: GameRef))
        .getOrElse(MoveStats.empty)

    new SubEntry(
      if (stats.total > 0) moves + (move -> stats) else moves - move,
      gameRefs.filterNot(_.gameId == game.gameId)
    )
  }

  def writeStats(out: OutputStream) = {
    writeUint(out, moves.size)
    moves.foreach {
      case (move, stats) =>
        writeUci(out, move)
        stats.write(out)
    }
  }

  def write(out: OutputStream) = {
    writeStats(out)

    gameRefs.sortWith(_.averageRating > _.averageRating)
      .distinct
      .take(SubEntry.maxTopGames)
      .foreach(_.write(out))
  }
}

object SubEntry extends PackHelper {

  val maxTopGames = 4

  def empty = new SubEntry(Map.empty, List.empty)

  def fromGameRef(game: GameRef, move: Either[Uci.Move, Uci.Drop]) =
    empty.withGameRef(game, move)

  def fromExistingGameRef(game: GameRef) =
    empty.withExistingGameRef(game)

  def readStats(in: InputStream, gameRefs: List[GameRef] = List.empty): SubEntry = {
    var remainingMoves = readUint(in)
    val moves = scala.collection.mutable.Map.empty[Either[Uci.Move, Uci.Drop], MoveStats]
    while (remainingMoves > 0) {
      moves += (readUci(in) -> MoveStats.read(in))
      remainingMoves -= 1;
    }
    new SubEntry(moves.toMap, gameRefs)
  }

  def read(in: InputStream) = {
    val subEntry = readStats(in)

    val gameRefs = scala.collection.mutable.ListBuffer.empty[GameRef]
    while (in.available > 0) {
      gameRefs += GameRef.read(in)
    }

    subEntry.copy(gameRefs = gameRefs.toList)
  }
}
