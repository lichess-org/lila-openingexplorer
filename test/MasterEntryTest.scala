package lila.openingexplorer

import java.io.{ ByteArrayInputStream, ByteArrayOutputStream }

import org.specs2.mutable._

import chess.{ Color, Pos }
import chess.format.Uci

class MasterEntryTest extends Specification {

  private def pipe(entry: MasterEntry): MasterEntry  = {
    val out = new ByteArrayOutputStream()
    entry.write(out)

    val in = new ByteArrayInputStream(out.toByteArray)
    MasterEntry.read(in)
  }

  "master entry writing" should {

    "pack a single game" in {
      val ref = GameRef("ref00000", Some(Color.White), SpeedGroup.Blitz, 1230)
      val entry = MasterEntry.fromGameRef(ref, Left(Uci.Move(Pos.E2, Pos.E4)))

      pipe(entry).games mustEqual List(ref)
    }

    "pack two games" in {
      val move = Left(Uci.Move(Pos.D2, Pos.D4))
      val g1 = GameRef("g0000001", Some(Color.Black), SpeedGroup.Classical, 2300)
      val g2 = GameRef("g0000002", None, SpeedGroup.Classical, 2455)
      val entry = MasterEntry.fromGameRef(g1, move).withGameRef(g2, move)

      pipe(entry).games mustEqual List(g2, g1)
    }
  }
}
