package lila.openingexplorer

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

  def situationDrops(situation: chess.Situation): List[chess.Drop] =
    (for {
      role <- situation.board.crazyData.map(_.pockets(situation.color).roles.distinct).getOrElse(List.empty)
      pos <- situation.drops.getOrElse(List.empty)
    } yield situation.drop(role, pos).toList).flatten

  def situationMovesOrDrops(situation: chess.Situation): List[chess.MoveOrDrop] =
    situationMoves(situation).map(Left(_)) ::: situationDrops(situation).map(Right(_))
}
