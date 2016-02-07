package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.{KyotoDb, WritableVisitor}
import fm.last.commons.kyoto.factory.{KyotoDbBuilder, Mode}

import chess.{Hash, Situation, Move, PositionHash}

class MasterDatabase extends MasterDatabasePacker {

  val hash = new Hash(32)  // 128 bit Zobrist hasher

  private val file = new File("data/master.kct")

  file.createNewFile

  private val db =
    new KyotoDbBuilder(file)
      .modes(Mode.CREATE, Mode.READ_WRITE)
      .buckets(2000000L * 40)
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

  def mergeAll(hashes: Set[PositionHash], gameRef: GameRef) = {
    db.accept(hashes.toArray, new WritableVisitor {
      def record(key: PositionHash, value: Array[Byte]): Array[Byte] = {
        pack(unpack(value).withGameRef(gameRef))
      }

      def emptyRecord(key: PositionHash): Array[Byte] = {
        pack(SubEntry.fromGameRef(gameRef))
      }
    })
  }

  def close = db.close()

}

object MasterDatabase {

  val maxGames = 5

}
