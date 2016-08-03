package lila.openingexplorer

import java.io.File
import java.io.{ ByteArrayInputStream, ByteArrayOutputStream }

import fm.last.commons.kyoto.{ KyotoDb, WritableVisitor }

import chess.{ Hash, Situation, MoveOrDrop, PositionHash }

final class MasterDatabase {

  private val db = Util.wrapLog(
    "Loading master database...",
    "Master database loaded!") {
      Kyoto.builder(Config.explorer.master.kyoto).buildAndOpen
    }

  def query(situation: Situation, maxMoves: Int, maxGames: Int): MasterQueryResult = {
    val entry = probe(situation)
    new MasterQueryResult(
      entry.totalWhite,
      entry.totalDraws,
      entry.totalBlack,
      entry.averageRating,
      entry.moves.toList.sortBy(-_._2.total).take(maxMoves).flatMap { case (uci, stats) =>
        val move = uci.left.map( m => situation.move(m.orig, m.dest, m.promotion))
                      .right.map( d => situation.drop(d.role, d.pos))

        move match {
          case Left(scalaz.Success(move)) => Some(Left(move) -> stats)
          case Right(scalaz.Success(drop)) => Some(Right(drop) -> stats)
          case _ => None
        }
      },
      List.empty
    )
  }

  def probe(situation: Situation): MasterEntry = probe(MasterDatabase.hash(situation))

  private def probe(h: PositionHash): MasterEntry = {
    Option(db.get(h)) match {
      case Some(bytes) => unpack(bytes)
      case None        => MasterEntry.empty
    }
  }

  private def unpack(b: Array[Byte]): MasterEntry = {
    val in = new ByteArrayInputStream(b)
    MasterEntry.read(in)
  }

  def merge(gameRef: GameRef, move: MoveOrDrop) = {
    val hash = MasterDatabase.hash(move.fold(_.situationBefore, _.situationBefore))
    val uci = move.left.map(_.toUci).right.map(_.toUci)

    db.accept(hash, new WritableVisitor {
      def record(key: PositionHash, value: Array[Byte]): Array[Byte] = {
        val out = new ByteArrayOutputStream()
        unpack(value).withGameRef(gameRef, uci).write(out)
        out.toByteArray
      }

      def emptyRecord(key: PositionHash): Array[Byte] = {
        val out = new ByteArrayOutputStream()
        MasterEntry.fromGameRef(gameRef, uci).write(out)
        out.toByteArray
      }
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
