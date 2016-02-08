package lila.openingexplorer

trait LichessDatabasePacker extends PackHelper {

  protected def pack(entry: Entry): Array[Byte] = {
    if (entry.totalGames == 0)
      Array.empty
    else if (entry.totalGames == 1)
      entry.selectAll.recentGames.head.pack
    else
      Array(1.toByte) ++ entry.selectAll.recentGames.map(_.pack).flatten
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
    }
  }

}

object LichessDatabasePacker {

  val maxGames = 5

}
