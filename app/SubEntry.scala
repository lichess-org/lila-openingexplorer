package lila.openingexplorer

import chess.format.Uci
import java.io.{ OutputStream, InputStream, ByteArrayInputStream }

case class SubEntry(
    moves: Map[Either[Uci.Move, Uci.Drop], MoveStats],
    games: List[GameRef]) extends PackHelper {

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
      game :: games)

  def withExistingGameRef(game: GameRef) = copy(games = game :: games)

  def write(out: OutputStream) = {
    writeUint(out, moves.size)
    moves.foreach { case (move, stats) =>
      writeUci(out, move)
      stats.write(out)
    }

    games.sortWith(_.averageRating > _.averageRating)
      .take(SubEntry.maxTopGames)
      .foreach(_.write(out))
  }
}

object SubEntry extends PackHelper {

  val maxTopGames = 4

  def empty = new SubEntry(Map.empty, List.empty)

  def fromGameRef(game: GameRef, move: Either[Uci.Move, Uci.Drop]) =
    empty.withGameRef(game, move)

  def read(in: InputStream) = {
    var remainingMoves = readUint(in)
    val moves = scala.collection.mutable.Map.empty[Either[Uci.Move, Uci.Drop], MoveStats]
    while (remainingMoves > 0) {
      moves += (readUci(in) -> MoveStats.read(in))
      remainingMoves -= 1;
    }

    val games = scala.collection.mutable.ListBuffer.empty[GameRef]
    while (in.available > 0) {
      games += GameRef.read(in)
    }

    new SubEntry(moves.toMap, games.toList)
  }
}
