package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class MasterDatabasePackerTest extends Specification with MasterDatabasePacker {

  "master database packer" should {

    "pack a single game" in {
      val ref = GameRef("ref00000", Some(Color.White), SpeedGroup.Blitz, 1230)
      val entry = SubEntry.fromGameRef(ref)

      unpack(pack(entry)).gameRefs mustEqual List(ref)
    }

    "pack two games" in {
      val g1 = GameRef("g0000001", Some(Color.Black), SpeedGroup.Classical, 2300)
      val g2 = GameRef("g0000002", None, SpeedGroup.Classical, 2455)
      val entry = SubEntry.fromGameRef(g1).withGameRef(g2)

      unpack(pack(entry)).gameRefs mustEqual List(g2, g1)
    }

    "pack thousands of games" in {
      val e = new SubEntry(12345, 23456, 34567, 2016, List.empty)

      val restored = unpack(pack(e))
      restored.whiteWins mustEqual 12345
      restored.draws mustEqual 23456
      restored.blackWins mustEqual 34567
    }

    "pack millions of games" in {
      val g1 = GameRef("g0000001", None, SpeedGroup.Classical, 2222)
      val e = new SubEntry(1000999, 9222333, 12, 1234567890L, List(g1))

      val restored = unpack(pack(e))
      restored.whiteWins mustEqual 1000999
      restored.draws mustEqual 9222333
      restored.blackWins mustEqual 12
      restored.averageRatingSum mustEqual 1234567890L
      restored.gameRefs mustEqual List(g1)
    }

  }

}
