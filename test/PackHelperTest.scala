package lila.openingexplorer

import java.io.{ ByteArrayInputStream, ByteArrayOutputStream }
import org.specs2.mutable._
import chess.format.Uci
import chess.Pos
import chess.{ King, Rook }

class PackHelperTest extends Specification with PackHelper {

  def pipeMove(move: Either[Uci.Move, Uci.Drop]): Either[Uci.Move, Uci.Drop] = {
    val out = new ByteArrayOutputStream()
    writeUci(out, move)

    val in = new ByteArrayInputStream(out.toByteArray)
    readUci(in)
  }

  "the pack helper" should {
    "correctly pack moves" in {
      val move = Uci.Move(Pos.E2, Pos.E3)
      pipeMove(Left(move)) mustEqual Left(move)
    }

    "correctly pack promotions" in {
      val move = Uci.Move(Pos.A7, Pos.A8, Some(Rook))
      pipeMove(Left(move)) mustEqual Left(move)
    }

    "correctly pack drops" in {
      val drop = Uci.Drop(King, Pos.H3)
      pipeMove(Right(drop)) mustEqual Right(drop)
    }
  }

  List(7, 127, 128, 129, 254, 255, 256, 257, 1234, 864197252500L).foreach { x =>
    "correctly pack uint: " + x in {
      val out = new ByteArrayOutputStream()
      writeUint(out, x)

      val in = new ByteArrayInputStream(out.toByteArray)
      readUint(in) mustEqual x
    }
  }
}
