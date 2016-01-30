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
      (topGames ++ other.topGames).sorted.reverse.take(10)
    )
  }

  def totalGames(r: RatingGroup): Long =
    whiteWins.getOrElse(r, 0L) + draws.getOrElse(r, 0L) + blackWins.getOrElse(r, 0L)

  def totalWhiteWins: Long = whiteWins.values.sum
  def totalDraws: Long = draws.values.sum
  def totalBlackWins: Long = blackWins.values.sum

  def totalGames: Long = totalWhiteWins + totalDraws + totalBlackWins

  def totalWins(color: Color) = color.fold(totalWhiteWins, totalBlackWins)

  private def packUint16(v: Int): Array[Byte] =
    Array((0xff & (v >> 8)).toByte, (0xff & v).toByte)

  private def packUint32(v: Long): Array[Byte] =
    packUint16((0xffff & (v >> 16)).toInt) ++ packUint16((0xffff & v).toInt)

  private def packUint48(v: Long): Array[Byte] =
    packUint32(0xffffffffL & (v >> 32)) ++ packUint16((0xffff & v).toInt)

  private def packGameRef(r: String): Array[Byte] = {
    // Game references consist of 8 alphanumeric characters.
    // 8 bytes
    (r.toCharArray.map(_.toByte) ++ Array.fill[Byte](8)(0)).take(8)
  }

  private def packSingle: Array[Byte] = {
    val gameResult = List(
      whiteWins.size >= 1,
      draws.size >= 1,
      blackWins.size >= 1
    ).indexOf(true)

    val rating = topGames.head._1

    // 1 + 1 + 2 + 8 = 12 bytes
    Array(
      1.toByte,  // packing type
      gameResult.toByte
    ) ++ packUint16(rating) ++ packGameRef(topGames.head._2)
  }

  def pack: Array[Byte] = {
    if (totalGames == 0)
      Array.empty
    else if (totalGames == 1)
      packSingle
    else
      packSingle // todo
  }
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
