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

    "correctly pack a few hundred games" in {
      val e = new Entry(
        Map.empty,
        Map(RatingGroup.Group1600 -> 123),
        Map.empty,
        Set.empty
      )

      val restored = Entry.unpack(e.pack)
      restored.draws.getOrElse(RatingGroup.Group1400, 0) mustEqual 0
      restored.draws.getOrElse(RatingGroup.Group1600, 0) mustEqual 123
    }

    "correctly pack thousands of games" in {
      val g1 = GameRef("00000000", 3490, None)
      val g2 = GameRef("22222222", 50, Some(Color.Black))

      val e = new Entry(
        Map(RatingGroup.Group2800 -> 293),
        Map(RatingGroup.Group0 -> 2000, RatingGroup.Group2800 -> 23),
        Map(RatingGroup.Group0 -> 1337),
        Set(g1, g2)
      )

      val restored = Entry.unpack(e.pack)

      restored.topGames mustEqual Set(g1, g2)

      restored.whiteWins.getOrElse(RatingGroup.Group2800, 0) mustEqual 293
      restored.draws.getOrElse(RatingGroup.Group0, 0) mustEqual 2000
      restored.draws.getOrElse(RatingGroup.Group2800, 0) mustEqual 23
      restored.blackWins.getOrElse(RatingGroup.Group0, 0) mustEqual 1337
    }

    "correctly pack millions of games" in {
      val e = new Entry(
        Map.empty,
        Map.empty,
        Map(RatingGroup.Group1400 -> 400060400L),
        Set.empty
      )

      val restored = Entry.unpack(e.pack)
      restored.blackWins.getOrElse(RatingGroup.Group1400, 0) mustEqual 400060400L
      restored.blackWins.getOrElse(RatingGroup.Group2800, 0) mustEqual 0
    }

  }

}
