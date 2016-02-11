package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class LichessDatabasePackerTest extends Specification with LichessDatabasePacker {

  "lichess database packer" should {

    "pack a single game" in {
      val ref = GameRef("ref00000", Some(Color.White), SpeedGroup.Bullet, 1999)
      val entry = Entry.fromGameRef(ref)

      unpack(pack(entry)).allGameRefs mustEqual List(ref)
    }

    "pack two games" in {
      val g1 = GameRef("g0000001", Some(Color.Black), SpeedGroup.Classical, 2300)
      val g2 = GameRef("g0000002", None, SpeedGroup.Classical, 2455)
      val entry = Entry.fromGameRef(g1).withGameRef(g2)

      unpack(pack(entry)).allGameRefs mustEqual List(g2, g1)
    }

    "pack thousands of games" in {
      val g1 = GameRef("g0000001", Some(Color.White), SpeedGroup.Blitz, 2033)
      val subEntry = new SubEntry(98765, 54321, 12345, 123456789L, List(g1))
      val entry = new Entry(Map((RatingGroup.Group2000, SpeedGroup.Blitz) -> subEntry))
      val restored = unpack(pack(entry))

      restored.totalWhiteWins mustEqual 98765
      restored.totalDraws mustEqual 54321
      restored.totalBlackWins mustEqual 12345
      restored.totalAverageRatingSum mustEqual 123456789L

      restored.allGameRefs mustEqual List(g1)
    }

    "preserve chronological order" in {
      val g1 = new GameRef("g0000001", None, SpeedGroup.Classical, 2620)
      val g2 = new GameRef("g0000002", None, SpeedGroup.Classical, 2610)
      val g3 = new GameRef("g0000003", None, SpeedGroup.Classical, 2650)

      val entry = Entry.fromGameRef(g1).withGameRef(g2).withGameRef(g3)
      entry.allGameRefs mustEqual List(g3, g2, g1)

      unpack(pack(entry)).allGameRefs.take(LichessDatabasePacker.maxRecentGames) mustEqual
        List(g3, g2, g1).take(LichessDatabasePacker.maxRecentGames)
    }

    "save some top games" in {
      val topGame = GameRef("abcdefgh", Some(Color.Black), SpeedGroup.Classical, 2871)

      val low1 = GameRef("low00001", Some(Color.Black), SpeedGroup.Classical, 2501)
      val low2 = GameRef("low00002", Some(Color.Black), SpeedGroup.Classical, 2502)
      val low3 = GameRef("low00003", Some(Color.Black), SpeedGroup.Classical, 2503)
      val low4 = GameRef("low00004", Some(Color.Black), SpeedGroup.Classical, 2504)
      val low5 = GameRef("low00005", Some(Color.Black), SpeedGroup.Classical, 2505)
      val low6 = GameRef("low00006", Some(Color.Black), SpeedGroup.Classical, 2506)
      val low7 = GameRef("low00007", Some(Color.Black), SpeedGroup.Classical, 2507)
      val low8 = GameRef("low00008", Some(Color.Black), SpeedGroup.Classical, 2508)
      val low9 = GameRef("low00009", Some(Color.Black), SpeedGroup.Classical, 2509)

      val subEntry =
        new SubEntry(
          123456789L, 234567890L, 345678901L, 864197252500L,
          List(low1, low2, low3, low4, low5, low6, low7, topGame, low8, low9))

      val entry = new Entry(Map((RatingGroup.Group2500, SpeedGroup.Classical) -> subEntry))
      val restored = unpack(pack(entry))

      restored.allGameRefs must contain(topGame)

      restored.totalWhiteWins mustEqual 123456789L
      restored.totalAverageRatingSum mustEqual 864197252500L
    }

  }

}
