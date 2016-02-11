package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.factory.{ KyotoDbBuilder, Mode, Compressor, PageComparator }
import fm.last.commons.kyoto.KyotoDb

final class GameInfoDatabase extends MasterDatabasePacker {

  private val dbFile = new File("data/game-info.kct")
  dbFile.createNewFile

  private val db =
    new KyotoDbBuilder(dbFile)
      .modes(Mode.CREATE, Mode.READ_WRITE)
      .buckets(2000000L)
      .compressor(Compressor.LZMA)
      .pageComparator(PageComparator.LEXICAL)
      .buildAndOpen

  def get(gameId: String): Option[GameInfo] =
    Option(db.get(gameId)) flatMap GameInfoDatabase.unpack

  def store(gameId: String, info: GameInfo) =
    db.set(gameId, GameInfoDatabase pack info)

  def close = {
    db.close()
  }
}

object GameInfoDatabase {

  def pack(info: GameInfo): String = List(
    info.white.name,
    info.white.rating,
    info.black.name,
    info.black.rating,
    info.year.fold("?")(_.toString)
  ) mkString "|"

  def unpack(str: String): Option[GameInfo] = str split '|' match {
    case Array(wn, wrS, bn, brS, yearS) => for {
      wr <- parseIntOption(wrS)
      br <- parseIntOption(brS)
      year = parseIntOption(yearS)
    } yield GameInfo(
      white = GameInfo.Player(wn, wr),
      black = GameInfo.Player(bn, br),
      year = year)
    case _ => None
  }
}
