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
      (topGames ++ other.topGames).toList.sortWith(_.rating > _.rating).take(Entry.maxGames).toSet
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

  def sumWhiteWins(ratingGroups: List[RatingGroup]): Long =
    ratingGroups.map(whiteWins.getOrElse(_, 0L)).sum

  def sumDraws(ratingGroups: List[RatingGroup]): Long =
    ratingGroups.map(draws.getOrElse(_, 0L)).sum

  def sumBlackWins(ratingGroups: List[RatingGroup]): Long =
    ratingGroups.map(blackWins.getOrElse(_, 0L)).sum

  def sumGames(ratingGroups: List[RatingGroup]): Long =
    ratingGroups.map(totalGames).sum

  def pack: Array[Byte] = {
    if (totalGames == 0)
      Array.empty
    else if (totalGames == 1)
      topGames.head.pack
    else if (totalGames <= Entry.maxGames)
      Array(1.toByte) ++
      takeTopGames(Entry.maxGames).map(_.pack).flatten
    else if (totalGames <= 256)
      Array(2.toByte) ++
      RatingGroup.all.map({
        case group =>
          Array(
            whiteWins.getOrElse(group, 0L).toByte,
            draws.getOrElse(group, 0L).toByte,
            blackWins.getOrElse(group, 0L).toByte
          )
      }).flatten ++
      takeTopGames(Entry.maxGames).map(_.pack).flatten
    else if (totalGames <= 65536)
      Array(3.toByte) ++
      RatingGroup.all.map({
        case group =>
          packUint16(whiteWins.getOrElse(group, 0L).toInt) ++
          packUint16(draws.getOrElse(group, 0L).toInt) ++
          packUint16(blackWins.getOrElse(group, 0L).toInt)
      }).flatten ++
      takeTopGames(Entry.maxGames).map(_.pack).flatten
    else
      Array(4.toByte) ++
      RatingGroup.all.map({
        case group =>
          packUint48(whiteWins.getOrElse(group, 0)) ++
          packUint48(draws.getOrElse(group, 0)) ++
          packUint48(blackWins.getOrElse(group, 0))
      }).flatten ++
      takeTopGames(Entry.maxGames).map(_.pack).flatten
  }

}

object Entry extends PackHelper {

  val maxGames = 5

  def empty: Entry =
    new Entry(Map.empty, Map.empty, Map.empty, Set.empty)

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
    if (b.size == GameRef.packSize) {
      fromGameRef(GameRef.unpack(b))
    } else b(0) match {
      case 1 =>
        b.drop(1)
          .grouped(GameRef.packSize)
          .map(GameRef.unpack _)
          .foldLeft(empty)({
            case (l, r) => l.combine(fromGameRef(r))
          })
      case 2 =>
        new Entry(
          RatingGroup.all.zipWithIndex.map({
            case (group, i) => group -> unpackUint8(b.drop(1 + i * 3 * 1)).toLong
          }).toMap,
          RatingGroup.all.zipWithIndex.map({
            case (group, i) => group -> unpackUint8(b.drop(1 + 1 + i * 3 * 1)).toLong
          }).toMap,
          RatingGroup.all.zipWithIndex.map({
            case (group, i) => group -> unpackUint8(b.drop(1 + 2 + i * 3 * 1)).toLong
          }).toMap,
          b.drop(1 + RatingGroup.all.size * 3 * 1)
            .grouped(GameRef.packSize)
            .map(GameRef.unpack _)
            .toSet
        )
      case 3 =>
        new Entry(
          RatingGroup.all.zipWithIndex.map({
            case (group, i) => group -> unpackUint16(b.drop(1 + i * 3 * 2)).toLong
          }).toMap,
          RatingGroup.all.zipWithIndex.map({
            case (group, i) => group -> unpackUint16(b.drop(1 + 2 + i * 3 * 2)).toLong
          }).toMap,
          RatingGroup.all.zipWithIndex.map({
            case (group, i) => group -> unpackUint16(b.drop(1 + 4 + i * 3 * 2)).toLong
          }).toMap,
          b.drop(1 + RatingGroup.all.size * 3 * 2)
            .grouped(GameRef.packSize)
            .map(GameRef.unpack _)
            .toSet
        )
      case 4 =>
        new Entry(
          RatingGroup.all.zipWithIndex.map({
            case (group, i) => group -> unpackUint48(b.drop(1 + i * 3 * 6))
          }).toMap,
          RatingGroup.all.zipWithIndex.map({
            case (group, i) => group -> unpackUint48(b.drop(1 + 6 + i * 3 * 6))
          }).toMap,
          RatingGroup.all.zipWithIndex.map({
            case (group, i) => group -> unpackUint48(b.drop(1 + 12 + i * 3 * 6))
          }).toMap,
          b.drop(1 + RatingGroup.all.size * 3 * 6)
            .grouped(GameRef.packSize)
            .map(GameRef.unpack _)
            .toSet
        )
    }
  }

}
