package lila.openingexplorer

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
}
