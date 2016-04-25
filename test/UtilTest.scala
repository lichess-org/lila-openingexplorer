package lila.openingexplorer

import org.specs2.mutable._

import chess.{ Drop, Pos, Pawn, Knight, Queen }
import chess.format.Forsyth
import chess.variant.{ Standard, Crazyhouse }

class UtilTest extends Specification {

  "Util" should {

    "generate drops" in {
      val fen = "r3k2r/pppq1ppp/2n1p3/3n2B1/3P4/4PP2/PP1QBP1P/R3K1R1/BPBpnn w Qkq - 22 12"
      val situation = ((Forsyth << fen) get) withVariant Crazyhouse

      val drop = situation.drop(Pawn, Pos.C3).toOption.get

      Util.situationDrops(situation) must contain(drop)
    }

    "generate underpromotions" in {
      val fen = "rnbqk1nr/ppp2ppp/8/4P3/1BP5/8/PP2KpPP/RN1Q1BNR b kq - 1 7"
      val situation = ((Forsyth << fen) get) withVariant Standard

      val fxg1N = situation.move(Pos.F2, Pos.G1, Some(Knight)).toOption.get
      val fxg1Q = situation.move(Pos.F2, Pos.G1, Some(Queen)).toOption.get

      Util.situationMoves(situation) must contain(fxg1N)
      Util.situationMoves(situation) must contain(fxg1Q)
    }

    "uniquify hashes" in {
      val a = Array(1.toByte, 2.toByte, 3.toByte)
      val b = a.toList.toArray

      Util.distinctHashes(List(a, b)) mustEqual Array(a)
      Util.distinctHashes(List(b, a)) mustEqual Array(a)
    }
  }
}
