package lila.openingexplorer

import org.specs2.mutable._

import chess.{ Drop, Pos, Pawn, Knight, Queen }
import chess.format.Forsyth
import chess.variant.{ Standard, Crazyhouse }

class UtilTest extends Specification {

  "Util" should {

    "uniquify hashes" in {
      val a = Array(1.toByte, 2.toByte, 3.toByte)
      val b = a.toList.toArray

      Util.distinctHashes(List(a, b)) mustEqual Array(a)
      Util.distinctHashes(List(b, a)) mustEqual Array(a)
    }
  }
}
