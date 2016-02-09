package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.factory.{ KyotoDbBuilder, Mode, PageComparator }
import fm.last.commons.kyoto.{ KyotoDb, WritableVisitor }

import chess.variant.Variant
import chess.{ Hash, PositionHash, Situation, MoveOrDrop }

final class LichessDatabase extends LichessDatabasePacker {

  private val variants = Variant.all.filter(chess.variant.FromPosition!=)

  private val dbs: Map[Variant, KyotoDb] = variants.map({
    case variant =>
      val file = new File(s"data/${variant.key}.kct")
      file.createNewFile
      val db =
        new KyotoDbBuilder(file)
          .modes(Mode.CREATE, Mode.READ_WRITE)
          .pageComparator(PageComparator.LEXICAL)
          .buckets(5000000L * MAX_PLIES / 2) // at least 10% of expected records
          .buildAndOpen

      variant -> db
  }).toMap

  import LichessDatabase.Request

  def probe(situation: Situation, request: Request): SubEntry =
    probe(situation.board.variant, LichessDatabase.hash(situation), request)

  private def probe(variant: Variant, h: PositionHash, request: Request): SubEntry = {
    dbs.get(variant).flatMap(db => Option(db.get(h))) match {
      case Some(bytes) => unpack(bytes).select(request.ratings, request.speeds)
      case None        => SubEntry.empty
    }
  }

  def probeChildren(situation: Situation, request: Request): List[(MoveOrDrop, SubEntry)] =
    Util.situationMovesOrDrops(situation).map { move =>
      move -> probe(move.fold(_.situationAfter, _.situationAfter), request)
    }.toList

  def merge(variant: Variant, gameRef: GameRef, hashes: Array[PositionHash]) = dbs get variant foreach { db =>
    val freshRecord = pack(Entry.fromGameRef(gameRef))

    db.accept(hashes, new WritableVisitor {
      def record(key: PositionHash, value: Array[Byte]): Array[Byte] = {
        pack(unpack(value).withGameRef(gameRef))
      }

      def emptyRecord(key: PositionHash): Array[Byte] = freshRecord
    })
  }

  def closeAll = {
    dbs.values.foreach { db =>
      db.close()
    }
  }
}

object LichessDatabase {

  case class Request(
    speeds: List[SpeedGroup],
    ratings: List[RatingGroup])

  val hash = new Hash(32) // 128 bit Zobrist hasher

}
