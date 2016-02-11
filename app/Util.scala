package lila.openingexplorer

import scala.collection.mutable.WrappedArray

import scalaz.Validation.FlatMap._

object Util {

  // deduplicates castling moves
  def situationMoves(situation: chess.Situation): List[chess.Move] =
    situation.moves.values.flatten.foldLeft(List.empty[chess.Move] -> Set.empty[chess.Pos]) {
      case ((list, seenRooks), move) => move.castle match {
        case Some((_, (rookPos, _))) =>
          if (seenRooks(rookPos)) (list, seenRooks)
          else (move :: list, seenRooks + rookPos)
        case _ => (move :: list, seenRooks)
      }
    }._1

  def situationDrops(situation: chess.Situation): List[chess.Drop] = {
    val droppablePositions = situation.drops.getOrElse(chess.Pos.all filterNot situation.board.pieces.contains)
    (for {
      role <- situation.board.crazyData.map(_.pockets(situation.color).roles.distinct).getOrElse(List.empty)
      pos <- droppablePositions
    } yield situation.drop(role, pos).toOption).flatten
  }

  def situationMovesOrDrops(situation: chess.Situation): List[chess.MoveOrDrop] =
    situationMoves(situation).map(Left(_)) ::: situationDrops(situation).map(Right(_))

  def distinctHashes(hashes: List[chess.PositionHash]): Array[chess.PositionHash] =
    hashes.map(h => (h: WrappedArray[Byte])).distinct.map(_.array).toArray

  def wrapLog[A](before: String, after: String)(f: => A): A = {
    println(before)
    val res = f
    println(after)
    res
  }

}
