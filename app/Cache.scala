package lila.openingexplorer

import play.api.cache.SyncCacheApi
import scala.concurrent.duration._

import chess.variant.Variant

final class Cache(cache: SyncCacheApi) {

  private val config = Config.explorer.cache
  private val statConfig = Config.explorer.statCache

  def master(data: Forms.master.Data)(computation: => String): String =
    fenMoveNumber(data.fen).fold(computation) { moveNumber =>
      if (moveNumber > config.maxMoves) computation
      else cache.getOrElseUpdate(s"master:${data.fen}:${data.topGamesOrDefault}", config.ttl)(computation)
    }

  def lichess(data: Forms.lichess.Data)(computation: => String): String =
    fenMoveNumber(data.fen).fold(computation) { moveNumber =>
      if (moveNumber > config.maxMoves) computation
      if (!data.fullHouse) computation
      else cache.getOrElseUpdate(s"${data.actualVariant.key}:${data.fen}:${data.topGamesOrDefault}", config.ttl)(computation)
    }

  def stat(computation: => String): String =
    cache.getOrElseUpdate(s"stat", statConfig.ttl)(computation)

  private def fenMoveNumber(fen: String) =
    fen split ' ' lift 5 flatMap parseIntOption
}
