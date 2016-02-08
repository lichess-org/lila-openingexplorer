package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class MasterDatabasePackerTest extends Specification with MasterDatabasePacker {

  "master database packer" should {

    "pack a single game" in {
      val ref = GameRef("ref00000", Some(Color.White), SpeedGroup.Blitz, 1230)
      val entry = SubEntry.fromGameRef(ref)

      unpack(pack(entry)).topGames mustEqual List(ref)
    }

    "pack two games" in {
      val g1 = GameRef("g0000001", Some(Color.Black), SpeedGroup.Classical, 2300)
      val g2 = GameRef("g0000002", None, SpeedGroup.Classical, 2455)
      val entry = SubEntry.fromGameRef(g1).withGameRef(g2)

      unpack(pack(entry)).topGames.toSet mustEqual Set(g1, g2)
    }

    "pack thousands of games" in {
      val e = new SubEntry(12345, 23456, 34567, 2016, List.empty, List.empty)

      val restored = unpack(pack(e))
      restored.whiteWins mustEqual 12345
      restored.draws mustEqual 23456
      restored.blackWins mustEqual 34567
    }

  }

}
