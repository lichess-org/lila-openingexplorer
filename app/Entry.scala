package lila.openingexplorer

case class Entry(sub: Map[Tuple2[RatingGroup, SpeedGroup], SubEntry]) {

  def subEntry(ratingGroup: RatingGroup, speedGroup: SpeedGroup): SubEntry =
    sub.getOrElse((ratingGroup, speedGroup), SubEntry.empty)

  def totalGames = sub.values.map(_.totalGames).sum

  def withGameRef(game: GameRef): Entry = {
    RatingGroup.find(game.averageRating) match {
      case Some(ratingGroup) =>
        copy(sub = sub + ((ratingGroup, game.speed) -> subEntry(ratingGroup, game.speed).withGameRef(game)))
      case None =>
        this  // rating too low
    }
  }

  def select(ratingGroups: List[RatingGroup], speedGroups: List[SpeedGroup]): SubEntry =
    // cross product
    (for {
      ratingGroup <- ratingGroups
      speedGroup <- speedGroups
    } yield subEntry(ratingGroup, speedGroup))
      .foldLeft(SubEntry.empty)((l, r) => l.combine(r))

  def selectAll = select(RatingGroup.all, SpeedGroup.all)

}

object Entry {

  def empty = Entry(Map.empty)

  def fromGameRef(game: GameRef) = Entry.empty.withGameRef(game)

}
