package lila.openingexplorer

import chess.{ MoveOrDrop, Situation }
import chess.format.Uci

object Util {

  def moveFromUci(situation: Situation, uci: Either[Uci.Move, Uci.Drop]): Option[MoveOrDrop] = {
    val move = uci.left
      .map(m => situation.move(m.orig, m.dest, m.promotion))
      .map(d => situation.drop(d.role, d.pos))

    move match {
      case Left(scalaz.Success(move))  => Some(Left(move))
      case Right(scalaz.Success(drop)) => Some(Right(drop))
      case _                           => None
    }
  }

  def wrapLog[A](before: String, after: String)(f: => A): A = {
    val start = System.currentTimeMillis
    println(before)
    val res      = f
    val duration = System.currentTimeMillis - start
    println(f"$after (${duration / 1000d}%.02f seconds)")
    res
  }
}
