package lila.openingexplorer

trait LichessDatabasePacker extends PackHelper {

  protected def pack(entry: Entry): Array[Byte] = {
    if (entry.totalGames == 0)
      Array.empty
    else if (entry.totalGames == 1)
      entry.selectAll.recentGames.head.pack
    else if (entry.totalGames < 5)  // todo: calculate optimum
      Array(1.toByte) ++ entry.selectAll.recentGames.map(_.pack).flatten
    else if (entry.maxPerWinnerAndGroup < 256)
      packMulti(entry, 2, packUint8)
    else if (entry.maxPerWinnerAndGroup < 65536)
      packMulti(entry, 3, packUint16)
    else if (entry.maxPerWinnerAndGroup < 4294967296L)
      packMulti(entry, 4, packUint32)
    else
      packMulti(entry, 5, packUint48)
  }

  private def packMulti(
      entry: Entry,
      meta: Int,
      helper: Long => Array[Byte]): Array[Byte] = {
    val sampleGames =
      entry.sub.values.map(_.recentGames.take(LichessDatabasePacker.maxRecentGames)).flatten
      entry.selectAll.topGames.take(LichessDatabasePacker.maxTopGames)

    packUint8(meta) ++
      Entry.allGroups.map((g) => packSubEntry(entry.subEntry(g._1, g._2), helper)).flatten ++
      sampleGames.toList.distinct.map(_.pack).flatten
  }

  private def packSubEntry(
      subEntry: SubEntry,
      helper: Long => Array[Byte]): Array[Byte] = {
    helper(subEntry.whiteWins) ++
      helper(subEntry.draws) ++
      helper(subEntry.blackWins) ++
      packUint48(subEntry.averageRatingSum)
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
        unpackMulti(b, unpackUint8, 1)
      case 3 =>
        unpackMulti(b, unpackUint16, 2)
      case 4 =>
        unpackMulti(b, unpackUint32, 4)
      case 8 =>
        unpackMulti(b, unpackUint48, 6)
    }
  }

  private def unpackMulti(
      b: Array[Byte],
      helper: Array[Byte] => Long,
      width: Int): Entry = {
    // unpack aggregated stats
    val entry = new Entry(Entry.allGroups.zipWithIndex.map({ case (g, i) =>
      g -> unpackSubEntry(b.drop(1 + i * (width + width + width + 6)), helper, width)
    }).toMap)

    // unpack games
    b.drop(1 + Entry.allGroups.size * (width + width + width + 6))
      .grouped(GameRef.packSize)
      .map(GameRef.unpack _)
      .foldRight(entry)((l, r) => r.withExistingGameRef(l))
  }

  private def unpackSubEntry(
      b: Array[Byte],
      helper: Array[Byte] => Long,
      width: Int): SubEntry = {
    new SubEntry(
      helper(b),
      helper(b.drop(width)),
      helper(b.drop(width + width)),
      unpackUint48(b.drop(width + width + width)),
      List.empty, List.empty
    )
  }

}

object LichessDatabasePacker {

  val maxTopGames = 5

  val maxRecentGames = 5

}
