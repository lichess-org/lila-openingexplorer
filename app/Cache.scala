package lila.openingexplorer

import play.api.cache.CacheApi
import scala.concurrent.duration._

import chess.variant.Variant

final class Cache(cache: CacheApi) {

  private val maxMoveNumber = 2
  private val ttl = 10 minutes

  def master(fen: String)(computation: => String): String =
    fenMoveNumber(fen).fold(computation) { moveNumber =>
      if (moveNumber > maxMoveNumber) computation
      else cache.getOrElse(s"master:$fen", ttl)(computation)
    }

  def lichess(data: Forms.lichess.Data)(computation: => String): String =
    fenMoveNumber(data.fen).fold(computation) { moveNumber =>
      if (moveNumber > maxMoveNumber) computation
      if (!data.fullHouse) computation
      else cache.getOrElse(s"${data.actualVariant.key}:${data.fen}", ttl)(computation)
    }

  private def fenMoveNumber(fen: String) =
    fen split ' ' lift 5 flatMap parseIntOption
}
