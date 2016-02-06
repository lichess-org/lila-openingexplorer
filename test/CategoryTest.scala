package lila.openingexplorer

import org.specs2.mutable._

class CategoryTest extends Specification {

  "categories" should {

    "be found" in {
      Category.find("threecheck") mustEqual Some(Category.ThreeCheck)
      Category.find("bullet") mustEqual Some(Category.Bullet)
    }

    "not be found" in {
      Category.find("foo") mustEqual None
    }

  }

}
