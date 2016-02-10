package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class LichessDatabasePackerTest extends Specification with LichessDatabasePacker {

  "lichess database packer" should {

    "pack a single game" in {
      val ref = GameRef("ref00000", Some(Color.White), SpeedGroup.Bullet, 1999)
      val entry = Entry.fromGameRef(ref)

      unpack(pack(entry)).selectAll.topGames mustEqual List(ref)
    }

    "pack two games" in {
      val g1 = GameRef("g0000001", Some(Color.Black), SpeedGroup.Classical, 2300)
      val g2 = GameRef("g0000002", None, SpeedGroup.Classical, 2455)
      val entry = Entry.fromGameRef(g1).withGameRef(g2)

      unpack(pack(entry)).selectAll.topGames.toSet mustEqual Set(g1, g2)
    }

    "pack thousands of games" in {
      val g1 = GameRef("g0000001", Some(Color.White), SpeedGroup.Blitz, 2033)
      val subEntry = new SubEntry(98765, 54321, 12345, 123456789L, List(g1), List(g1))
      val entry = new Entry(Map((RatingGroup.Group2000, SpeedGroup.Blitz) -> subEntry))
      val restored = unpack(pack(entry))

      restored.selectAll.whiteWins mustEqual 98765
      restored.selectAll.draws mustEqual 54321
      restored.selectAll.blackWins mustEqual 12345
      restored.selectAll.averageRatingSum mustEqual 123456789L

      restored.selectAll.recentGames mustEqual List(g1)
      restored.selectAll.topGames mustEqual List(g1)
    }

    "preserve chronological order" in {
      val g1 = new GameRef("g0000001", None, SpeedGroup.Classical, 2620)
      val g2 = new GameRef("g0000002", None, SpeedGroup.Classical, 2610)
      val g3 = new GameRef("g0000003", None, SpeedGroup.Classical, 2650)

      val entry = Entry.fromGameRef(g1).withGameRef(g2).withGameRef(g3)
      entry.selectAll.recentGames mustEqual List(g3, g2, g1)

      unpack(pack(entry)).selectAll.recentGames mustEqual List(g3, g2, g1)
    }

    "save some top games" in {
      val topGame = GameRef("abcdefgh", Some(Color.Black), SpeedGroup.Classical, 2871)
      val subEntry = new SubEntry(123456789L, 234567890L, 345678901L, 864197252500L, List(topGame), List.empty)
      val entry = new Entry(Map((RatingGroup.Group2500, SpeedGroup.Classical) -> subEntry))
      val restored = unpack(pack(entry))

      restored.selectAll.topGames mustEqual List(topGame)

      restored.selectAll.whiteWins mustEqual 123456789L
      restored.selectAll.averageRatingSum mustEqual 864197252500L
    }

  }

}
