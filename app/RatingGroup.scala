package lila.openingexplorer

case class RatingGroup private (min: Option[Int], max: Option[Int]) {

}

object RatingGroup {

  val Group2800 = RatingGroup(Some(2800), None)
  val Group2600 = RatingGroup(Some(2600), Some(2799))
  val Group2400 = RatingGroup(Some(2400), Some(2599))
  val Group2200 = RatingGroup(Some(2200), Some(2399))
  val Group2000 = RatingGroup(Some(2000), Some(2199))
  val Group1800 = RatingGroup(Some(1800), Some(1999))
  val Group1600 = RatingGroup(Some(1600), Some(1799))
  val Group1400 = RatingGroup(Some(1400), Some(1599))
  val Group1200 = RatingGroup(Some(1200), Some(1399))
  val Group1000 = RatingGroup(Some(1000), Some(1199))
  val Group0    = RatingGroup(None,       Some(999))

  val all = List(
    Group0, Group1000, Group1200, Group1400, Group1600, Group1800,
    Group2000, Group2200, Group2400, Group2600, Group2800
  )

  def find(whiteRating: Int, blackRating: Int): RatingGroup =
    find(math.min(whiteRating, blackRating))

  def find(rating: Int): RatingGroup = {
    all.find {
      case RatingGroup(None, Some(max))      =>                  rating <= max
      case RatingGroup(Some(min), Some(max)) => min <= rating && rating <= max
      case RatingGroup(Some(min), None)      => min <= rating
      case RatingGroup(None, None)           => false
    } get
  }

  def range(min: Option[Int], max: Option[Int]): List[RatingGroup] = {
    all
      .filter({
        group => min.map(_ <= group.max.getOrElse(5000)).getOrElse(true)
      })
      .filter({
        group => max.map(group.min.getOrElse(0) < _).getOrElse(true)
      })
  }

}
