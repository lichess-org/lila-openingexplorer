package lila.openingexplorer

import fm.last.commons.kyoto.factory.{ Compressor, PageComparator }
import javax.inject.{ Inject, Singleton }
import akka.actor.CoordinatedShutdown

@Singleton
final class GameInfoDatabase @Inject() (
    config: Config,
    shutdown: CoordinatedShutdown
)(implicit ec: scala.concurrent.ExecutionContext) {

  private val db = Util.wrapLog(
    "Loading gameInfo database...",
    "GameInfo database loaded!"
  ) {
    Kyoto
      .builder(config.explorer.gameInfo.kyoto)
      .compressor(Compressor.LZMA)
      .pageComparator(PageComparator.LEXICAL)
      .buildAndOpen
  }

  def get(gameId: String): Option[GameInfo] =
    Option(db.get(gameId)) flatMap GameInfoDatabase.unpack

  def contains(gameId: String): Boolean = db.exists(gameId)

  def store(gameId: String, info: GameInfo): Boolean =
    db.putIfAbsent(gameId, GameInfoDatabase pack info)

  def count = db.recordCount()

  shutdown.addTask(CoordinatedShutdown.PhaseServiceStop, "close master db") { () =>
    scala.concurrent.Future {
      db.close()
      akka.Done
    }
  }
}

object GameInfoDatabase {

  def pack(info: GameInfo): String =
    List(
      info.white.name,
      info.white.rating,
      info.black.name,
      info.black.rating,
      info.year.fold("?")(_.toString)
    ) mkString "|"

  def unpack(str: String): Option[GameInfo] =
    str split '|' match {
      case Array(wn, wrS, bn, brS, yearS) =>
        for {
          wr <- wrS.toIntOption
          br <- brS.toIntOption
          year = yearS.toIntOption
        } yield GameInfo(
          white = GameInfo.Player(wn, wr),
          black = GameInfo.Player(bn, br),
          year = year
        )
      case _ => None
    }
}
