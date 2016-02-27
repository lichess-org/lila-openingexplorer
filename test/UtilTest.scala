package lila.openingexplorer

import org.specs2.mutable._

import chess.{ Drop, Pawn, Pos }
import chess.format.Forsyth
import chess.variant.Crazyhouse

class UtilTest extends Specification {

  "Util" should {

    "generate drops" in {
      val fen = "r3k2r/pppq1ppp/2n1p3/3n2B1/3P4/4PP2/PP1QBP1P/R3K1R1/BPBpnn w Qkq - 22 12"
      val situation = ((Forsyth << fen) get) withVariant Crazyhouse

      val drop = situation.drop(Pawn, Pos.C3).toOption.get

      Util.situationDrops(situation) must contain(drop)
    }

    "uniquify hashes" in {
      val a = Array(1.toByte, 2.toByte, 3.toByte)
      val b = a.toList.toArray

      Util.distinctHashes(List(a, b)) mustEqual Array(a)
      Util.distinctHashes(List(b, a)) mustEqual Array(a)
    }
  }
}
