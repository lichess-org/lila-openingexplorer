package lila.openingexplorer

trait LichessDatabasePacker extends PackHelper {

  protected def pack(entry: Entry): Array[Byte] = {
    if (entry.totalGames == 0)
      Array.empty
    else if (entry.totalGames == 1)
      entry.selectAll.recentGames.head.pack
    else if (entry.totalGames <= LichessDatabasePacker.maxPackFormat1)
      Array(1.toByte) ++ entry.selectAll.recentGames.map(_.pack).flatten
    else if (entry.maxPerWinnerAndGroup < MaxUint8)
      packMulti(entry, 2, packUint8, packUint24)
    else if (entry.maxPerWinnerAndGroup < MaxUint16)
      packMulti(entry, 3, packUint16, packUint32)
    else if (entry.maxPerWinnerAndGroup < MaxUint24)
      packMulti(entry, 4, packUint24, packUint48)
    else if (entry.maxPerWinnerAndGroup < MaxUint32)
      packMulti(entry, 5, packUint32, packUint48)
    else
      packMulti(entry, 6, packUint48, packUint48)
  }

  private def packMulti(
      entry: Entry,
      meta: Int,
      helper: Long => Array[Byte],
      ratingHelper: Long => Array[Byte]): Array[Byte] = {
    val sampleGames =
      entry.sub.values.map(_.recentGames.take(LichessDatabasePacker.maxRecentGames)).flatten ++
      entry.selectAll.topGames.take(LichessDatabasePacker.maxTopGames)

    packUint8(meta) ++
      Entry.allGroups.map((g) => packSubEntry(entry.subEntry(g._1, g._2), helper, ratingHelper)).flatten ++
      sampleGames.toList.distinct.map(_.pack).flatten
  }

  private def packSubEntry(
      subEntry: SubEntry,
      helper: Long => Array[Byte],
      ratingHelper: Long => Array[Byte]): Array[Byte] = {
    helper(subEntry.whiteWins) ++
      helper(subEntry.draws) ++
      helper(subEntry.blackWins) ++
      ratingHelper(subEntry.averageRatingSum)
  }

  protected def unpack(b: Array[Byte]): Entry = {
    if (b.size == 0) {
      Entry.empty
    } else if (b.size == GameRef.packSize) {
      Entry.fromGameRef(GameRef.unpack(b))
    } else b(0) match {
      case 1 =>
        b.drop(1)
          .grouped(GameRef.packSize)
          .map(GameRef.unpack _)
          .foldRight(Entry.empty)({
            case (l, r) => r.withGameRef(l)
          })
      case 2 =>
        unpackMulti(b, unpackUint8, 1, unpackUint24, 3)
      case 3 =>
        unpackMulti(b, unpackUint16, 2, unpackUint32, 4)
      case 4 =>
        unpackMulti(b, unpackUint24, 3, unpackUint48, 6)
      case 5 =>
        unpackMulti(b, unpackUint32, 4, unpackUint48, 6)
      case 6 =>
        unpackMulti(b, unpackUint48, 6, unpackUint48, 6)
    }
  }

  private def unpackMulti(
      b: Array[Byte],
      helper: Array[Byte] => Long,
      width: Int,
      ratingHelper: Array[Byte] => Long,
      ratingWidth: Int): Entry = {
    // unpack aggregated stats
    val entry = new Entry(Entry.allGroups.zipWithIndex.map({ case (g, i) =>
      g -> unpackSubEntry(b.drop(1 + i * (width + width + width + ratingWidth)), helper, width, ratingHelper)
    }).toMap)

    // unpack games
    b.drop(1 + Entry.allGroups.size * (width + width + width + ratingWidth))
      .grouped(GameRef.packSize)
      .map(GameRef.unpack _)
      .foldRight(entry)((l, r) => r.withExistingGameRef(l))
  }

  private def unpackSubEntry(
      b: Array[Byte],
      helper: Array[Byte] => Long,
      width: Int,
      ratingHelper: Array[Byte] => Long): SubEntry = {
    new SubEntry(
      helper(b),
      helper(b.drop(width)),
      helper(b.drop(width + width)),
      ratingHelper(b.drop(width + width + width)),
      List.empty, List.empty
    )
  }

}

object LichessDatabasePacker {

  val maxTopGames = 4

  val maxRecentGames = 2

  val maxPackFormat1 = maxTopGames + Entry.allGroups.size * (1 + 1 + 1 + 3 + maxRecentGames * GameRef.packSize) / GameRef.packSize

}
