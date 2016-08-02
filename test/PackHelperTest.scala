package lila.openingexplorer

import java.io.{ ByteArrayOutputStream, ByteArrayInputStream }
import org.specs2.mutable._

class PackHelperTest extends Specification with PackHelper {

  "the pack helper" should {
    "correctly pack 24bit integers" in {
      unpackUint24(packUint24(12345)) mustEqual 12345
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
