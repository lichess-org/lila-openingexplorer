package lila.openingexplorer

import java.io.{ ByteArrayOutputStream, ByteArrayInputStream }
import org.specs2.mutable._
import chess.format.Uci
import chess.Pos
import chess.Rook

class PackHelperTest extends Specification with PackHelper {

  def pipeMove(move: Uci.Move): Uci.Move = {
    val out = new ByteArrayOutputStream()
    writeMove(out, move)

    val in = new ByteArrayInputStream(out.toByteArray)
    readMove(in)
  }

  "the pack helper" should {
    "correctly pack 24bit integers" in {
      unpackUint24(packUint24(12345)) mustEqual 12345
    }

    "correctly pack moves" in {
      val move = Uci.Move(Pos.E2, Pos.E3)
      pipeMove(move) mustEqual move
    }

    "correctly pack promotions" in {
      val move = Uci.Move(Pos.A7, Pos.A8, Some(Rook))
      pipeMove(move) mustEqual move
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
