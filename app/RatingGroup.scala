package lila.openingexplorer

sealed abstract class RatingGroup(val range: Range) {}

object RatingGroup {

  case object Group1600 extends RatingGroup(1600 to 1799)
  case object Group1800 extends RatingGroup(1800 to 1999)
  case object Group2000 extends RatingGroup(2000 to 2200)
  case object Group2200 extends RatingGroup(2200 to 2499)
  case object Group2500 extends RatingGroup(2500 to Int.MaxValue)

  val all = List(Group1600, Group1800, Group2000, Group2200, Group2500)

  def find(averageRating: Int): RatingGroup =
    all.find(_.range contains averageRating) getOrElse Group1600
}
