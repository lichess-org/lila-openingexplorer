package lila.openingexplorer

import chess.Color

case class SubEntry(
    whiteWins: Long,
    draws: Long,
    blackWins: Long,
    averageRatingSum: Long,
    topGames: List[GameRef],
    recentGames: List[GameRef]) {

  def totalGames = whiteWins + draws + blackWins

  def withGameRef(game: GameRef): SubEntry = {
    val intermediate = copy(
      averageRatingSum = averageRatingSum + game.averageRating,
      topGames =
        (game :: topGames)
          .sortWith(_.averageRating > _.averageRating)
          .take(SubEntry.maxGames)
    )

    game.winner match {
      case Some(Color.White) => intermediate.copy(whiteWins = whiteWins + 1)
      case Some(Color.Black) => intermediate.copy(blackWins = blackWins + 1)
      case None              => intermediate.copy(draws = draws + 1)
    }
  }

  def withRecentGameRef(game: GameRef): SubEntry = {
    withGameRef(game).copy(
     recentGames = (game :: recentGames).take(SubEntry.maxGames)
    )
  }

}

object SubEntry {

  val maxGames = 5

  def empty = new SubEntry(0, 0, 0, 0, List.empty, List.empty)

  def fromGameRef(game: GameRef) = empty.withGameRef(game)

  def fromRecentGameRef(game: GameRef) = empty.withRecentGameRef(game)

}
