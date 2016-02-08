package lila.openingexplorer

import org.specs2.mutable._

class PackHelperTest extends Specification with PackHelper {

  "the pack helper" should {
    "correctly pack 24bit integers" in {
      unpackUint24(packUint24(12345)) mustEqual 12345
    }
  }

}
