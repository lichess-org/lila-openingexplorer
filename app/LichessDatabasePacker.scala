package lila.openingexplorer

import java.io.{ OutputStream, ByteArrayOutputStream, ByteArrayInputStream }

trait LichessDatabasePacker extends PackHelper {

  protected def pack(entry: Entry): Array[Byte] = {
    val out = new ByteArrayOutputStream()

    if (entry.totalGames == 0) { }
    else if (entry.totalGames == 1 && entry.allGameRefs.size == 1)
      entry.allGameRefs.head.write(out)
    else if (entry.totalGames <= LichessDatabasePacker.maxPackFormat1 &&
             entry.totalGames == entry.allGameRefs.size) {
      // all games explicitly known by ref
      out.write(1)
      entry.allGameRefs.foreach(_.write(out))
    }
    else packVariable(out, 7, entry)

    out.toByteArray
  }

  private def packVariable(out: OutputStream, meta: Int, entry: Entry) = {
    val exampleGames =
      entry.sub.values.flatMap(_.gameRefs.take(LichessDatabasePacker.maxRecentGames)) ++
      SpeedGroup.all.flatMap { speed =>
        entry.gameRefs(Entry.groups(speed))
          .sortWith(_.averageRating > _.averageRating)
          .take(LichessDatabasePacker.maxTopGames)
      }

    out.write(meta)

    Entry.allGroups.foreach { g =>
      val subEntry = entry.subEntry(g._1, g._2)
      writeUint(out, subEntry.whiteWins)
      writeUint(out, subEntry.draws)
      writeUint(out, subEntry.blackWins)
      writeUint(out, subEntry.averageRatingSum)
    }

    exampleGames.toList.distinct.foreach(_.write(out))
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
      case 7 =>
        unpackVariable(b)
    }
  }

  private def unpackVariable(b: Array[Byte]): Entry = {
    val in = new ByteArrayInputStream(b)
    in.read()

    val subEntries = scala.collection.mutable.Map.empty[(RatingGroup, SpeedGroup), SubEntry]
    Entry.allGroups.foreach { g =>
      val white = readUint(in)
      val draws = readUint(in)
      val black = readUint(in)
      val averageRatingSum = readUint(in)
      subEntries += g -> SubEntry(white, draws, black, averageRatingSum, List.empty)
    }

    val games = scala.collection.mutable.ListBuffer.empty[GameRef]
    while (in.available() > 0) {
      games += GameRef.read(in)
    }

    games.foldRight(new Entry(subEntries.toMap))((l, r) => r.withExistingGameRef(l))
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
      List.empty
    )
  }
}

object LichessDatabasePacker {

  val maxTopGames = 4

  val maxRecentGames = 2

  val maxPackFormat1 = (Entry.allGroups.size * (1 + 1 + 1 + 3 + maxRecentGames * GameRef.packSize) + SpeedGroup.all.size * maxTopGames * GameRef.packSize) / GameRef.packSize
}
