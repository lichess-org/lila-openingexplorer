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

  def withExistingGameRef(game: GameRef): Entry = {
    RatingGroup.find(game.averageRating) match {
      case Some(ratingGroup) =>
        new Entry(sub + ((ratingGroup, game.speed) -> subEntry(ratingGroup, game.speed).withExistingGameRef(game)))
      case None =>
        this  // rating too low
    }
  }

  def gameRefs: List[GameRef] =
    sub.values.flatMap(_.gameRefs).toList

  def select(ratings: List[RatingGroup], speeds: List[SpeedGroup]): SubEntry =
    selectGroups(Entry.groups(ratings, speeds))

  def selectAll: SubEntry = selectGroups(Entry.allGroups)

  def selectGroups(groups: List[(RatingGroup, SpeedGroup)]): SubEntry = {
    val subEntries = groups.map((g) => subEntry(g._1, g._2))

    new SubEntry(
      subEntries.map(_.whiteWins).sum,
      subEntries.map(_.draws).sum,
      subEntries.map(_.blackWins).sum,
      subEntries.map(_.averageRatingSum).sum,
      // interleave recent game refs
      subEntries.map(_.gameRefs).flatMap(_.zipWithIndex).sortBy(_._2).map(_._1)
    )
  }

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
