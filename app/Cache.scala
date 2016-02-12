package lila.openingexplorer

import play.api.cache.CacheApi
import scala.concurrent.duration._

import chess.variant.Variant

final class Cache(cache: CacheApi) {

  private val config = Config.explorer.cache

  def master(fen: String)(computation: => String): String =
    fenMoveNumber(fen).fold(computation) { moveNumber =>
      if (moveNumber > config.maxMoves) computation
      else cache.getOrElse(s"master:$fen", config.ttl)(computation)
    }

  def lichess(data: Forms.lichess.Data)(computation: => String): String =
    fenMoveNumber(data.fen).fold(computation) { moveNumber =>
      if (moveNumber > config.maxMoves) computation
      if (!data.fullHouse) computation
      else cache.getOrElse(s"${data.actualVariant.key}:${data.fen}", config.ttl)(computation)
    }

  private def fenMoveNumber(fen: String) =
    fen split ' ' lift 5 flatMap parseIntOption
}
