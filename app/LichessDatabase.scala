package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.factory.{ KyotoDbBuilder, Mode }
import fm.last.commons.kyoto.KyotoDb

import chess.variant.Variant
import chess.{ Hash, PositionHash, Situation, Move }

final class LichessDatabase {

  private val variants = Variant.all.filter(chess.variant.FromPosition!=)

  private val dbs = variants.map({
    case variant =>
      val file = new File(s"data/${variant.key}.kct")
      file.createNewFile
      val db =
        new KyotoDbBuilder(file)
          .modes(Mode.CREATE, Mode.READ_WRITE)
          .buckets(140000000L * 40 / 2) // at least 10% of expected records
          .buildAndOpen

      variant -> db
  }).toMap

  import LichessDatabase.Request

  def probe(situation: Situation, request: Request): SubEntry = ???

  def probeChildren(situation: Situation, request: Request): List[(Move, SubEntry)] = ???

  def merge(variant: Variant, gameRef: GameRef, hashes: Set[PositionHash]) = ???

  def closeAll = {
    dbs.values.foreach { db =>
      db.close()
    }
  }
}

object LichessDatabase {

  case class Request(
    variant: Variant,
    speeds: List[SpeedGroup],
    ratings: List[RatingGroup])

  val hash = new Hash(32) // 128 bit Zobrist hasher

  val maxGames = 5
}
