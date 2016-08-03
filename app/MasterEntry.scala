package lila.openingexplorer

import chess.format.Uci
import java.io.{ OutputStream, InputStream, ByteArrayInputStream }

case class MasterEntry(
    moves: Map[Either[Uci.Move, Uci.Drop], MoveStats],
    games: List[GameRef]) extends PackHelper {

  def withGameRef(game: GameRef, move: Either[Uci.Move, Uci.Drop]) =
    new MasterEntry(
      moves + (move -> moves.getOrElse(move, MoveStats.empty).withGameRef(game)),
      game :: games)

  def write(out: OutputStream) = {
    writeUint(out, moves.size)
    moves.foreach { case (move, stats) =>
      writeUci(out, move)
      stats.write(out)
    }

    games.sortWith(_.averageRating > _.averageRating)
      .take(MasterEntry.maxTopGames)
      .foreach(_.write(out))
  }
}

object MasterEntry extends PackHelper {

  val maxTopGames = 4

  def empty = new MasterEntry(Map.empty, List.empty)

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

    new MasterEntry(moves.toMap, games.toList)
  }
}
