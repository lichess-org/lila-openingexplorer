package lila.openingexplorer

trait MasterDatabasePacker extends PackHelper {

  protected def pack(entry: SubEntry): Array[Byte] = {
    if (entry.totalGames == 0)
      Array.empty
    else if (entry.totalGames == 1 && entry.gameRefs.size == 1)
      entry.gameRefs.head.pack
    else if (entry.totalGames <= MasterDatabasePacker.maxPackFormat1 &&
             entry.gameRefs.size == entry.totalGames)
      // all game refs are explicitly known
      Array(1.toByte) ++ entry.gameRefs.map(_.pack).flatten
    else if (entry.maxPerWinner < MaxUint8)
      packMulti(entry, 2, packUint8)
    else if (entry.maxPerWinner < MaxUint16)
      packMulti(entry, 3, packUint16)
    else if (entry.maxPerWinner < MaxUint24)
      packMulti(entry, 4, packUint24)
    else if (entry.maxPerWinner < MaxUint32)
      packMulti(entry, 5, packUint32)
    else
      packMulti(entry, 6, packUint48)
  }

  private def packMulti(
      entry: SubEntry,
      meta: Int,
      helper: Long => Array[Byte]): Array[Byte] = {
    val exampleGames =
      entry.gameRefs
        .sortWith(_.averageRating > _.averageRating)
        .take(MasterDatabasePacker.maxTopGames)

    packUint8(meta) ++
      helper(entry.whiteWins) ++
      helper(entry.draws) ++
      helper(entry.blackWins) ++
      packUint48(entry.averageRatingSum) ++
      exampleGames.map(_.pack).flatten
  }

  protected def unpack(b: Array[Byte]): SubEntry = {
    if (b.size == 0) {
      SubEntry.empty
    } else if (b.size == GameRef.packSize) {
      SubEntry.fromGameRef(GameRef.unpack(b))
    } else b(0) match {
      case 1 =>
        b.drop(1)
          .grouped(GameRef.packSize)
          .map(GameRef.unpack _)
          .foldRight(SubEntry.empty)({
            case (l, r) => r.withGameRef(l)
          })
      case 2 =>
        unpackMulti(b, unpackUint8, 1)
      case 3 =>
        unpackMulti(b, unpackUint16, 2)
      case 4 =>
        unpackMulti(b, unpackUint24, 3)
      case 5 =>
        unpackMulti(b, unpackUint32, 4)
      case 6 =>
        unpackMulti(b, unpackUint48, 6)
    }
  }

  private def unpackMulti(
      b: Array[Byte],
      helper: Array[Byte] => Long,
      width: Int): SubEntry = {
    new SubEntry(
      helper(b.drop(1)),
      helper(b.drop(1 + width)),
      helper(b.drop(1 + width + width)),
      unpackUint48(b.drop(1 + width + width + width)),
      b.drop(1 + width + width + width + 6)
        .grouped(GameRef.packSize)
        .map(GameRef.unpack _)
        .toList
    )
  }

}

object MasterDatabasePacker {

  val maxTopGames = 4

  val maxPackFormat1 = maxTopGames + (1 + 1 + 1 + 6) / GameRef.packSize

}
