package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class EntryTest extends Specification {
  "entries" should {
    "be combinable" in {
      val e1 = Entry.fromGameRef(GameRef("g1", 1350, Some(Color.White)))
      val e2 = Entry.fromGameRef(GameRef("g2", 1110, Some(Color.White)))
      val e3 = Entry.fromGameRef(GameRef("g3", 2400, Some(Color.Black)))

      e1.totalGames mustEqual 1
      e1.combine(e2).totalGames mustEqual 2
      e1.combine(e2).combine(e3).totalGames mustEqual 3
      e1.combine(e2).combine(e3).totalBlackWins mustEqual 1
    }

    "correctly pack single games" in {
      val e = Entry.fromGameRef(GameRef("abcdefgh", 1234, None))
      Entry.unpack(e.pack) mustEqual e
    }

    "correctly pack two games" in {
      val e1 = Entry.fromGameRef(GameRef("abcdefgh", 1234, None))
      val e2 = Entry.fromGameRef(GameRef("12345678", 2345, Some(Color.White)))
      val e = e1.combine(e2)
      Entry.unpack(e.pack) mustEqual e
    }
  }
}
