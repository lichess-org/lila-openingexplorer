package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.{KyotoDb, WritableVisitor}
import fm.last.commons.kyoto.factory.{KyotoDbBuilder, Mode, Compressor, PageComparator}

import chess.{Hash, Situation, Move, PositionHash}

class MasterDatabase extends MasterDatabasePacker {

  val hash = new Hash(32)  // 128 bit Zobrist hasher

  private val dbFile = new File("data/master.kct")
  dbFile.createNewFile

  private val db =
    new KyotoDbBuilder(dbFile)
      .modes(Mode.CREATE, Mode.READ_WRITE)
      .pageComparator(PageComparator.LEXICAL)
      .buckets(2000000L * 40)
      .buildAndOpen

  private val pgnFile = new File("data/master-pgn.kct")
  pgnFile.createNewFile

  private val pgnDb =
    new KyotoDbBuilder(pgnFile)
      .modes(Mode.CREATE, Mode.READ_WRITE)
      .buckets(2000000L)
      .compressor(Compressor.LZMA)
      .pageComparator(PageComparator.LEXICAL)
      .buildAndOpen

  def probe(situation: Situation): SubEntry = probe(hash(situation))

  private def probe(h: PositionHash): SubEntry = {
    Option(db.get(h)) match {
      case Some(bytes) => unpack(bytes)
      case None        => SubEntry.empty
    }
  }

  def probeChildren(situation: Situation): List[(Move, SubEntry)] =
    Util.situationMoves(situation).map { move =>
      move -> probe(move.situationAfter)
    }.toList

  def merge(gameRef: GameRef, hashes: Set[PositionHash]) = {
    val freshRecord = pack(SubEntry.fromGameRef(gameRef))

    db.accept(hashes.toArray, new WritableVisitor {
      def record(key: PositionHash, value: Array[Byte]): Array[Byte] = {
        pack(unpack(value).withGameRef(gameRef))
      }

      def emptyRecord(key: PositionHash): Array[Byte] = freshRecord
    })
  }

  def getPgn(gameId: String): Option[String] = Option(pgnDb.get(gameId))

  def storePgn(gameId: String, pgn: String) = pgnDb.set(gameId, pgn)

  def close = {
    db.close()
    pgnDb.close()
  }

}

object MasterDatabase {

  val maxGames = 5

}
