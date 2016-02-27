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

    "save some top games per speed group" in {
      val topGame = GameRef("hgfedcba", None, SpeedGroup.Classical, 2555)

      // other classical games
      val recent1 = GameRef("recent01", None, SpeedGroup.Classical, 2501)
      val recent2 = GameRef("recent02", None, SpeedGroup.Classical, 2502)
      val recent3 = GameRef("recent03", None, SpeedGroup.Classical, 2503)
      val recent4 = GameRef("recent04", Some(Color.Black), SpeedGroup.Classical, 2504)
      val recent5 = GameRef("recent05", None, SpeedGroup.Classical, 2505)
      val recent6 = GameRef("recent06", None, SpeedGroup.Classical, 2506)
      val recent7 = GameRef("recent07", Some(Color.White), SpeedGroup.Classical, 2507)
      val recent8 = GameRef("recent08", None, SpeedGroup.Classical, 2508)
      val recent9 = GameRef("recent09", None, SpeedGroup.Classical, 2509)

      // higher ratings, but bullet
      val better1 = GameRef("better01", Some(Color.Black), SpeedGroup.Bullet, 2777)
      val better2 = GameRef("better02", Some(Color.Black), SpeedGroup.Bullet, 2778)
      val better3 = GameRef("better03", Some(Color.Black), SpeedGroup.Bullet, 2779)
      val better4 = GameRef("better04", Some(Color.White), SpeedGroup.Bullet, 2780)
      val better5 = GameRef("better05", Some(Color.White), SpeedGroup.Bullet, 2781)
      val better6 = GameRef("better06", None, SpeedGroup.Bullet, 2782)
      val better7 = GameRef("better07", None, SpeedGroup.Bullet, 2783)
      val better8 = GameRef("better08", Some(Color.White), SpeedGroup.Bullet, 2784)
      val better9 = GameRef("better08", Some(Color.White), SpeedGroup.Bullet, 2785)

      val entry = new Entry(Map(
        (RatingGroup.Group2500, SpeedGroup.Classical) -> new SubEntry(
          12345L, 23456L, 34567L, 456789L,
          List(recent1, recent2, recent3, topGame, recent4, recent5, recent6, recent7, recent8, recent9)
        ),
        (RatingGroup.Group2500, SpeedGroup.Bullet) -> new SubEntry(
          54321L, 65432L, 76543L, 98765L,
          List(better1, better2, better3, better4, better5, better6, better7, better8, better9)
        )
      ))

      val restored = unpack(pack(entry))

      restored.allGameRefs must contain(topGame)
    }
  }
}
