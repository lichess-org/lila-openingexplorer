package lila.openingexplorer

import chess.Color

case class Entry(
    whiteWins: Map[RatingGroup, Long],
    draws: Map[RatingGroup, Long],
    blackWins: Map[RatingGroup, Long],
    topGames: Set[GameRef]) extends PackHelper {

  def takeTopGames(n: Int) =
    topGames.toList.sortWith(_.rating > _.rating).take(n)

  def totalGames(r: RatingGroup): Long =
    whiteWins.getOrElse(r, 0L) +
      draws.getOrElse(r, 0L) +
      blackWins.getOrElse(r, 0L)

  def totalWhiteWins: Long = whiteWins.values.sum
  def totalDraws: Long = draws.values.sum
  def totalBlackWins: Long = blackWins.values.sum

  def totalGames: Long = totalWhiteWins + totalDraws + totalBlackWins

  def sumWhiteWins(ratingGroups: List[RatingGroup]): Long =
    ratingGroups.map(whiteWins.getOrElse(_, 0L)).sum

  def sumDraws(ratingGroups: List[RatingGroup]): Long =
    ratingGroups.map(draws.getOrElse(_, 0L)).sum

  def sumBlackWins(ratingGroups: List[RatingGroup]): Long =
    ratingGroups.map(blackWins.getOrElse(_, 0L)).sum

  def sumGames(ratingGroups: List[RatingGroup]): Long =
    ratingGroups.map(totalGames).sum

  def nonEmpty(ratingGroups: List[RatingGroup]) =
    sumGames(ratingGroups) > 0

  def withGameRef(game: GameRef): Entry = {
    val group = RatingGroup.find(game.rating)

    val newTopGames =
      (topGames.toList ++ List(game))
        .sortWith(_.rating > _.rating)
        .take(Entry.maxGames)
        .toSet

    game.winner match {
      case Some(Color.White) =>
        new Entry(
          whiteWins + (group -> (whiteWins.getOrElse(group, 0L) + 1L)),
          draws,
          blackWins,
          newTopGames
        )
      case Some(Color.Black) =>
        new Entry(
          whiteWins,
          draws,
          blackWins + (group -> (blackWins.getOrElse(group, 0L) + 1L)),
          newTopGames
        )
      case None =>
        new Entry(
          whiteWins,
          draws + (group -> (draws.getOrElse(group, 0L) + 1L)),
          blackWins,
          newTopGames
        )
    }
  }

  private def maxNumber: Long =
    // the maximum integer in the entry, used to decide required bit-length
    (whiteWins.values ++ blackWins.values ++ draws.values).max

  private def packMulti(
      format: Byte,
      helper: Long => Array[Byte]): Array[Byte] = {
    Array(format) ++
      RatingGroup.all.map({
        case group =>
          helper(whiteWins.getOrElse(group, 0L)) ++
            helper(draws.getOrElse(group, 0L)) ++
            helper(blackWins.getOrElse(group, 0L))
      }).flatten ++
      takeTopGames(Entry.maxGames).map(_.pack).flatten
  }

  def pack: Array[Byte] = {
    if (totalGames == 0)
      Array.empty
    else if (totalGames == 1)
      topGames.head.pack
    else if (totalGames <= Entry.maxGames)
      Array(1.toByte) ++
        takeTopGames(Entry.maxGames).map(_.pack).flatten
    else if (maxNumber < 256)
      packMulti(2, packUint8)
    else if (maxNumber < 65536)
      packMulti(3, packUint16)
    else if (maxNumber < 4294967296L)
      packMulti(4, packUint32)
    else
      packMulti(5, packUint48)
  }

}

object Entry extends PackHelper {

  val maxGames = 5

  def empty: Entry =
    new Entry(Map.empty, Map.empty, Map.empty, Set.empty)

  def fromGameRef(gameRef: GameRef): Entry =
    empty.withGameRef(gameRef)

  private def unpackMulti(
      b: Array[Byte],
      helper: Array[Byte] => Long,
      width: Int): Entry = {
    new Entry(
      RatingGroup.all.zipWithIndex.map({
        case (group, i) =>
          group -> helper(b.drop(1 + i * 3 * width)).toLong
      }).toMap,
      RatingGroup.all.zipWithIndex.map({
        case (group, i) =>
          group -> helper(b.drop(1 + width + i * 3 * width)).toLong
      }).toMap,
      RatingGroup.all.zipWithIndex.map({
        case (group, i) =>
          group -> helper(b.drop(1 + 2 * width + i * 3 * width)).toLong
      }).toMap,
      b.drop(1 + RatingGroup.all.size * 3 * width)
        .grouped(GameRef.packSize)
        .map(GameRef.unpack _)
        .toSet
    )
  }

  def unpack(b: Array[Byte]): Entry = {
    if (b.size == GameRef.packSize) {
      empty.withGameRef(GameRef.unpack(b))
    } else b(0) match {
      case 1 =>
        b.drop(1)
          .grouped(GameRef.packSize)
          .map(GameRef.unpack _)
          .foldLeft(empty)({
            case (l, r) => l.withGameRef(r)
          })
      case 2 =>
        unpackMulti(b, unpackUint8, 1)
      case 3 =>
        unpackMulti(b, unpackUint16, 2)
      case 4 =>
        unpackMulti(b, unpackUint32, 4)
      case 5 =>
        unpackMulti(b, unpackUint48, 6)
    }
  }

}
