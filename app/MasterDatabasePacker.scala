package lila.openingexplorer

import java.io.{ OutputStream, ByteArrayOutputStream, ByteArrayInputStream }

trait MasterDatabasePacker extends PackHelper {

  protected def pack(entry: SubEntry): Array[Byte] = {
    val out = new ByteArrayOutputStream()

    if (entry.totalGames == 0) { }
    else if (entry.totalGames == 1 && entry.gameRefs.size == 1)
      entry.gameRefs.head.write(out)
    else if (entry.totalGames <= MasterDatabasePacker.maxPackFormat1 &&
             entry.gameRefs.size == entry.totalGames) {
      // all game refs are explicitly known
      out.write(1)
      entry.gameRefs.foreach(_.write(out))
    }
    else packVariable(out, 7, entry)

    out.toByteArray
  }

  private def packVariable(out: OutputStream, meta: Int, entry: SubEntry) = {
    val exampleGames =
      entry.gameRefs
        .sortWith(_.averageRating > _.averageRating)
        .take(MasterDatabasePacker.maxTopGames)

    out.write(meta)
    writeUint(out, entry.whiteWins)
    writeUint(out, entry.draws)
    writeUint(out, entry.blackWins)
    writeUint(out, entry.averageRatingSum)
    exampleGames.foreach(_.write(out))
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
      case 7 =>
        unpackVariable(b)
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

  private def unpackVariable(b: Array[Byte]): SubEntry = {
    val in = new ByteArrayInputStream(b)
    in.read()
    val white = readUint(in)
    val draws = readUint(in)
    val black = readUint(in)
    val averageRatingSum = readUint(in)
    val games = scala.collection.mutable.ListBuffer.empty[GameRef]
    while (in.available() > 0) {
      games += GameRef.read(in)
    }
    new SubEntry(white, draws, black, averageRatingSum, games.toList)
  }
}

object MasterDatabasePacker {

  val maxTopGames = 4

  val maxPackFormat1 = maxTopGames + (1 + 1 + 1 + 6) / GameRef.packSize
}
