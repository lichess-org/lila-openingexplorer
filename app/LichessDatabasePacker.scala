package lila.openingexplorer

trait LichessDatabasePacker extends PackHelper {

  protected def pack(entry: Entry): Array[Byte] = ???

  protected def unpack(b: Array[Byte]): Entry = ???

}

object LichessDatabasePacker {

  val maxGames = 5

}
