package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class GameRefTest extends Specification {

  "GameRef packing" should {

    "be reversible" in {
      val g1 = GameRef("12abCD89", Some(Color.Black), SpeedGroup.Bullet, 3293)
      g1 mustEqual GameRef.unpack(g1.pack)

      val g2 = GameRef("89383928", Some(Color.White), SpeedGroup.Blitz, 2939)
      g2 mustEqual GameRef.unpack(g2.pack)

      val g3 = GameRef("ZZZZZZZZ", None, SpeedGroup.Classical, 4021)
      g3 mustEqual GameRef.unpack(g3.pack)

      val g4 = GameRef("zzzzzzzz", Some(Color.Black), SpeedGroup.Blitz, 29)
      g4 mustEqual GameRef.unpack(g4.pack)
    }

    "pad to 8 characters" in {
      val g = GameRef("00abcd00", Some(Color.White), SpeedGroup.Classical, 876)
      g mustEqual GameRef.unpack(g.pack)
    }

    "not overflow" in {
      val g = GameRef("abcdefgh", Some(Color.White), SpeedGroup.Blitz, 5555)
      val restored = GameRef.unpack(g.pack)

      restored.gameId mustEqual g.gameId
      restored.winner mustEqual g.winner
      restored.speed mustEqual g.speed
      restored.averageRating mustEqual 4095
    }
  }
}
