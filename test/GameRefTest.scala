package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class GameRefTest extends Specification {

  "GameRef packing" should {

    "be reversible" in {
      val g1 = GameRef("12abCD89", 12003, Some(Color.Black))
      g1 mustEqual GameRef.unpack(g1.pack)

      val g2 = GameRef("89383928", 2016, Some(Color.White))
      g2 mustEqual GameRef.unpack(g2.pack)

      val g3 = GameRef("ZZZZZZZZ", 4321, None)
      g3 mustEqual GameRef.unpack(g3.pack)
    }

  }

}
