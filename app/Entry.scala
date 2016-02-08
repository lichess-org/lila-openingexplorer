package lila.openingexplorer

case class Entry(sub: Map[(RatingGroup, SpeedGroup), SubEntry]) {

  def subEntry(ratingGroup: RatingGroup, speedGroup: SpeedGroup): SubEntry =
    sub.getOrElse((ratingGroup, speedGroup), SubEntry.empty)

  def totalGames = sub.values.map(_.totalGames).sum

  def maxPerWinnerAndGroup = sub.values.map(_.maxPerWinner).max

  def withGameRef(game: GameRef): Entry = {
    RatingGroup.find(game.averageRating) match {
      case Some(ratingGroup) =>
        copy(sub = sub + ((ratingGroup, game.speed) -> subEntry(ratingGroup, game.speed).withGameRef(game)))
      case None =>
        this  // rating too low
    }
  }

  def select(ratings: List[RatingGroup], speeds: List[SpeedGroup]): SubEntry =
    selectGroups(Entry.groups(ratings, speeds))

  def selectAll: SubEntry = selectGroups(Entry.allGroups)

  def selectGroups(groups: List[(RatingGroup, SpeedGroup)]): SubEntry =
    groups.map((g) => subEntry(g._1, g._2)).foldLeft(SubEntry.empty)((l, r) => l.combine(r))

}

object Entry {

  def empty = Entry(Map.empty)

  def fromGameRef(game: GameRef) = Entry.empty.withGameRef(game)

  def groups(
      ratings: List[RatingGroup],
      speeds: List[SpeedGroup]): List[(RatingGroup, SpeedGroup)] = {
    // cross product
    for {
      ratingGroup <- ratings
      speedGroup <- speeds
    } yield (ratingGroup, speedGroup)
  }

  val allGroups = groups(RatingGroup.all, SpeedGroup.all)

}
