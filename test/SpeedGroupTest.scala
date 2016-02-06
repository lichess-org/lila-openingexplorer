package lila.openingexplorer

import org.specs2.mutable._

class SpeedGroupTest extends Specification {

  "some speed groups" should {

    "be bullet" in {
      SpeedGroup.fromTimeControl("60+1") mustEqual SpeedGroup.Bullet
      SpeedGroup.fromTimeControl("60+0") mustEqual SpeedGroup.Bullet
    }

    "be blitz" in {
      SpeedGroup.fromTimeControl("180+0") mustEqual SpeedGroup.Blitz
      SpeedGroup.fromTimeControl("300+0") mustEqual SpeedGroup.Blitz
      SpeedGroup.fromTimeControl("300+2") mustEqual SpeedGroup.Blitz
    }

    "be classical" in {
      SpeedGroup.fromTimeControl("") mustEqual SpeedGroup.Classical
      SpeedGroup.fromTimeControl("-") mustEqual SpeedGroup.Classical
      SpeedGroup.fromTimeControl("600+0") mustEqual SpeedGroup.Classical
      SpeedGroup.fromTimeControl("900+10") mustEqual SpeedGroup.Classical
    }

  }

}
