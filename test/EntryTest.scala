package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class EntryTest extends Specification {
  "entries" should {
    "be combinable" in {
      val e1 = Entry.fromGame(Some(Color.White), 1350, 1100, "g1")
      val e2 = Entry.fromGame(Some(Color.White), 1110, 1120, "g2")
      val e3 = Entry.fromGame(Some(Color.Black), 1800, 2400, "g3")

      e1.totalGames mustEqual 1
      e1.combine(e2).totalGames mustEqual 2
      e1.combine(e2).combine(e3).totalGames mustEqual 3
      e1.combine(e2).combine(e3).totalBlackWins mustEqual 1
    }

    "correctly pack single games" in {
      val e = Entry.fromGame(None, 1234, 2345, "g4")
      e.pack mustEqual Array(
        1,
        1,
        0x0d, 0xfb,
        0x67, 0x34, 0, 0, 0, 0, 0, 0
      ).map(_.toByte)
    }
  }
}
