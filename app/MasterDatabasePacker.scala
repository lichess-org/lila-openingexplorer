package lila.openingexplorer

trait MasterDatabasePacker extends PackHelper {

  protected def pack(entry: SubEntry): Array[Byte] = {
    if (entry.totalGames == 0)
      Array.empty
    else if (entry.totalGames == 1)
      entry.recentGames.head.pack
    else if (entry.totalGames <= 6)  // carefully calculated boundary
      Array(1.toByte) ++ entry.recentGames.map(_.pack).flatten
    else if (entry.maxPerWinner < 256)
      packMulti(entry, 2, packUint8)
    else if (entry.maxPerWinner < 65536)
      packMulti(entry, 3, packUint16)
    else if (entry.maxPerWinner < 4294967296L)
      packMulti(entry, 4, packUint32)
    else
      packMulti(entry, 5, packUint48)
  }

  private def packMulti(
      entry: SubEntry,
      meta: Int,
      helper: Long => Array[Byte]): Array[Byte] = {
    packUint8(meta) ++
      helper(entry.whiteWins) ++
      helper(entry.draws) ++
      helper(entry.blackWins) ++
      packUint48(entry.averageRatingSum) ++
      entry.topGames.take(MasterDatabasePacker.maxTopGames).map(_.pack).flatten
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
        unpackMulti(b, unpackUint32, 4)
      case 5 =>
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
        .toList,
      List.empty
    )
  }

}

object MasterDatabasePacker {

  val maxTopGames = 5

}
