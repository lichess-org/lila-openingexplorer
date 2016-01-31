package lila.openingexplorer

import scalaz.Scalaz._

import chess.Color

case class Entry(
    whiteWins: Map[RatingGroup, Long],
    draws: Map[RatingGroup, Long],
    blackWins: Map[RatingGroup, Long],
    topGames: Set[GameRef]) extends PackHelper {

  def combine(other: Entry): Entry = {
    new Entry(
      whiteWins |+| other.whiteWins,
      draws |+| other.draws,
      blackWins |+| other.blackWins,
      (topGames ++ other.topGames).toList.sortWith(_.rating > _.rating).take(30)
    )
  }

  def totalGames(r: RatingGroup): Long =
    whiteWins.getOrElse(r, 0L) + draws.getOrElse(r, 0L) + blackWins.getOrElse(r, 0L)

  def takeTopGames(n: Int) =
    topGames.toList.sortWith(_.rating > _.rating).take(n)

  def totalWhiteWins: Long = whiteWins.values.sum
  def totalDraws: Long = draws.values.sum
  def totalBlackWins: Long = blackWins.values.sum

  def totalGames: Long = totalWhiteWins + totalDraws + totalBlackWins

  def totalWins(color: Color) = color.fold(totalWhiteWins, totalBlackWins)

  def pack: Array[Byte] = {
    if (totalGames == 0)
      Array.empty
    else if (totalGames == 1)
      topGames.head.pack
    else if (totalGames < 30)
      topGames.foldLeft(Array(1.toByte))(_ ++ _.pack)
    else
      RatingGroup.all.foldLeft(Array(2.toByte))({
        case (l, group) =>
          l ++ packUint48(whiteWins.getOrElse(group, 0))
            ++ packUint48(draws.getOrElse(group, 0))
               packUint48(blackWins.getOrElse(group, 0))
      }) ++ takeTopGames(5).foldLeft(Array.empty)(_ ++ _.pack)
  }
}

object Entry {
  def empty: Entry =
    new Entry(Map.empty, Map.empty, Map.empty, List.empty)

  def fromGameRef(gameRef: GameRef): Entry = {
    val ratingGroup = RatingGroup.find(gameRef.rating)

    gameRef.winner match {
      case Some(Color.White) =>
        new Entry(Map(ratingGroup -> 1), Map.empty, Map.empty, Set(gameRef))
      case Some(Color.Black) =>
        new Entry(Map.empty, Map.empty, Map(ratingGroup -> 1), Set(gameRef))
      case None =>
        new Entry(Map.empty, Map(ratingGroup -> 1), Map.empty, Set(gameRef))
    }
  }

  def unpack(b: Array[Byte]): Entry = {
    if (b.size == GameRef.pack_size) {
      fromGameRef(GameRef.unpack(b))
    } else b(0) match {
      case 1 =>
        b.drop(1)
          .grouped(GameRef.pack_size)
          .map(GameRef.unpack _)
          .foldLeft(empty)({
            case (l, r) => l.combine(fromGameRef(r))
          })
      case 2 =>
        b.drop(1).take(RatingGroup.all.size * (6 + 6 + 6))
          .grouped(6)
          .map(
    }
  }
}
