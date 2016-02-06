package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class SubEntryTest extends Specification {

  "sub entries" should {

    "be combinable" in {
      val g1 = new GameRef("g1", Some(Color.White), SpeedGroup.Blitz, 1350)
      val g2 = new GameRef("g2", Some(Color.Black), SpeedGroup.Blitz, 1110)
      val g3 = new GameRef("g3", None, SpeedGroup.Blitz, 2400)

      SubEntry.fromGameRef(g1).totalGames mustEqual 1
      SubEntry.fromGameRef(g1).withGameRef(g2).totalGames mustEqual 2
      SubEntry.fromGameRef(g1).withGameRef(g2).withGameRef(g3).totalGames mustEqual 3
      SubEntry.fromGameRef(g1).withGameRef(g2).withGameRef(g3).blackWins mustEqual 1
    }

  }

}
