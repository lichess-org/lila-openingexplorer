package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.KyotoDb
import fm.last.commons.kyoto.factory.{KyotoDbBuilder, Mode}

import chess.{Hash, Situation, Move}

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

  private def probe(h: Array[Byte]): SubEntry = {
    Option(db.get(h)) match {
      case Some(bytes) => unpack(bytes)
      case None        => SubEntry.empty
    }
  }

  def probeChildren(situation: Situation): List[(Move, SubEntry)] =
    situation.moves.values.flatten.map { move =>
      move -> probe(move.situationAfter)
    }.toList

  def merge(h: Array[Byte], gameRef: GameRef) =
    db.set(h, pack(probe(h).withGameRef(gameRef)))

  def close = db.close()

}

object MasterDatabase {

  val maxGames = 5

}
