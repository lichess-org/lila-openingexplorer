package lila.openingexplorer

import org.specs2.mutable._

class RatingGroupTest extends Specification {

  "rating groups" should {

    "be found" in {
      RatingGroup.find(1678) mustEqual RatingGroup.Group1600
      RatingGroup.find(2000) mustEqual RatingGroup.Group2000
    }

    "find the first group" in {
      RatingGroup.find(77) mustEqual RatingGroup.Group1600
    }

    "find the last group" in {
      RatingGroup.find(3002) mustEqual RatingGroup.Group2500
    }

  }

}
