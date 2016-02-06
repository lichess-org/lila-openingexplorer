package lila.openingexplorer

import org.specs2.mutable._

class RatingGroupTest extends Specification {

  "rating groups" should {

    "be found" in {
      RatingGroup.find(1678) mustEqual Some(RatingGroup.Group1600)
      RatingGroup.find(2000) mustEqual Some(RatingGroup.Group2000)
    }

    "find the first group" in {
      RatingGroup.find(77) mustEqual None
    }

    "find the last group" in {
      RatingGroup.find(3002) mustEqual Some(RatingGroup.Group2500)
    }

  }

}
