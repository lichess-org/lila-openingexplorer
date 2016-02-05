package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.KyotoDb
import fm.last.commons.kyoto.factory.{KyotoDbBuilder, Mode}

import chess.{Hash, Situation, Move}

class Database {

  val hash = new Hash(32)  // 128 bit Zobrist hasher

  private val dbs = Category.all.map({
    case category =>
      val file = new File(category.name ++ ".kct")
      file.createNewFile

      val db =
        new KyotoDbBuilder(file)
          .modes(Mode.CREATE, Mode.READ_WRITE)
          .buckets(140000000L * 60 / 2)  // at least 10% of expected records
          .buildAndOpen

     category -> db
  }).toMap

  def probe(category: Category, h: Array[Byte]): Entry = {
    Option(dbs(category).get(h)) match {
      case Some(bytes) => Entry.unpack(bytes)
      case None        => Entry.empty
    }
  }

  def probe(category: Category, situation: Situation): Entry =
    probe(category, hash(situation))

  def probeChildren(
      category: Category,
      situation: Situation): Map[Move, Entry] = {
    situation.moves.values.flatten.map {
      case (move) => move -> probe(category, move.situationAfter)
    }.toMap
  }

  def merge(category: Category, h: Array[Byte], gameRef: GameRef) = {
    dbs(category).set(h, probe(category, h).withGameRef(gameRef).pack)
  }

  def closeAll = {
    dbs.foreach {
      case (category, db) =>
        db.close()
    }
  }

}
