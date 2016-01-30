package lila.openingexplorer

import scalaz.Scalaz._

import chess.Color

case class Entry(
    whiteWins: Map[RatingGroup, Long],
    draws: Map[RatingGroup, Long],
    blackWins: Map[RatingGroup, Long],
    topGames: List[Tuple2[Int, String]]) {

  def combine(other: Entry): Entry = {
    new Entry(
      whiteWins |+| other.whiteWins,
      draws |+| other.draws,
      blackWins |+| other.blackWins,
      topGames ++ other.topGames
    )
  }

  def totalGames(r: RatingGroup): Long =
    whiteWins.getOrElse(r, 0L) + draws.getOrElse(r, 0L) + blackWins.getOrElse(r, 0L)

  def totalWhiteWins: Long = whiteWins.values.sum
  def totalDraws: Long = draws.values.sum
  def totalBlackWins: Long = blackWins.values.sum

  def totalGames: Long = totalWhiteWins + totalDraws + totalBlackWins
}

object Entry {
  def fromGame(winner: Option[Color], whiteRating: Int, blackRating: Int, gameRef: String) = {
    val ratingGroup = RatingGroup.find(whiteRating, blackRating)
    val topGame = List((whiteRating + blackRating, gameRef))

    winner match {
      case Some(Color.White) =>
        new Entry(Map(ratingGroup -> 1), Map.empty, Map.empty, topGame)
      case Some(Color.Black) =>
        new Entry(Map.empty, Map.empty, Map(ratingGroup -> 1), topGame)
      case None =>
        new Entry(Map.empty, Map(ratingGroup -> 1), Map.empty, topGame)
    }
  }
}
