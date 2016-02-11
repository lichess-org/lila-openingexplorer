package lila.openingexplorer

import chess.Color

case class SubEntry(
    whiteWins: Long,
    draws: Long,
    blackWins: Long,
    averageRatingSum: Long,
    gameRefs: List[GameRef]) {

  // game refs must be in chronological order, newer first
  def recentGames: List[GameRef] = gameRefs.take(SubEntry.maxRecentGames)

  def topGames: List[GameRef] =
    gameRefs
      .sortWith(_.averageRating > _.averageRating)
      .take(SubEntry.maxTopGames)

  def totalGames = whiteWins + draws + blackWins

  def averageRating: Int =
    if (totalGames == 0) 0 else (averageRatingSum / totalGames).toInt

  def maxPerWinner = math.max(math.max(whiteWins, blackWins), draws)

  def withExistingGameRef(game: GameRef): SubEntry =
    copy(gameRefs = game :: gameRefs)

  def withGameRef(game: GameRef): SubEntry = {
    val intermediate = copy(
      gameRefs = game :: gameRefs,
      averageRatingSum = averageRatingSum + game.averageRating
    )

    game.winner match {
      case Some(Color.White) => intermediate.copy(whiteWins = whiteWins + 1)
      case Some(Color.Black) => intermediate.copy(blackWins = blackWins + 1)
      case None              => intermediate.copy(draws = draws + 1)
    }
  }

}

object SubEntry {

  def empty = new SubEntry(0, 0, 0, 0, List.empty)

  def fromGameRef(game: GameRef) = empty.withGameRef(game)

  val maxTopGames = 4

  val maxRecentGames = 2

}
