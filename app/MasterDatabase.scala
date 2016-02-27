package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.factory.{ Mode, PageComparator }
import fm.last.commons.kyoto.{ KyotoDb, WritableVisitor }

import chess.{ Hash, Situation, MoveOrDrop, PositionHash }

final class MasterDatabase extends MasterDatabasePacker {

  private val db = Util.wrapLog(
    "Loading master database...",
    "Master database loaded!") {
      val config = Config.explorer.master
      val dbFile = new File(config.kyoto.file)
      dbFile.createNewFile
      Kyoto.builder(dbFile)
        .modes(Mode.CREATE, Mode.READ_WRITE)
        .buckets(config.kyoto.buckets)
        .memoryMapSize(config.kyoto.memoryMapSize)
        .defragUnitSize(config.kyoto.defragUnitSize)
        .buildAndOpen
    }

  private def probe(situation: Situation): SubEntry = probe(MasterDatabase.hash(situation))

  private def probe(h: PositionHash): SubEntry = {
    Option(db.get(h)) match {
      case Some(bytes) => unpack(bytes)
      case None        => SubEntry.empty
    }
  }

  def query(situation: Situation, topGames: Int = 0): QueryResult = {
    val entry = probe(situation)
    new QueryResult(
      entry.whiteWins,
      entry.draws,
      entry.blackWins,
      entry.averageRating,
      List.empty,
      entry.gameRefs
        .sortWith(_.averageRating > _.averageRating)
        .take(math.min(topGames, MasterDatabasePacker.maxTopGames)))
  }

  def queryChildren(situation: Situation): Children =
    Util.situationMovesOrDrops(situation).map { move =>
      move -> query(move.fold(_.situationAfter, _.situationAfter))
    }.toList

  def merge(gameRef: GameRef, hashes: Array[PositionHash]) = {
    val freshRecord = pack(SubEntry.fromGameRef(gameRef))

    db.accept(hashes, new WritableVisitor {
      def record(key: PositionHash, value: Array[Byte]): Array[Byte] = {
        pack(unpack(value).withGameRef(gameRef))
      }

      def emptyRecord(key: PositionHash): Array[Byte] = freshRecord
    })
  }

  def subtract(gameRef: GameRef, hashes: Array[PositionHash]) = {
    db.accept(hashes, new WritableVisitor {
      def record(key: PositionHash, value: Array[Byte]) = {
        val subtracted = unpack(value).withoutExistingGameRef(gameRef)
        if (subtracted.isEmpty) WritableVisitor.REMOVE else pack(subtracted)
      }

      // should not happen
      def emptyRecord(key: PositionHash): Array[Byte] = WritableVisitor.NOP
    })
  }

  def uniquePositions = db.recordCount()

  def close = {
    db.close()
  }

}

object MasterDatabase {

  val hash = new Hash(32) // 128 bit Zobrist hasher

}
