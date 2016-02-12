package lila.openingexplorer

case class QueryResult(
    whiteWins: Long,
    draws: Long,
    blackWins: Long,
    averageRating: Int,
    recentGames: List[GameRef],
    topGames: List[GameRef]) {

  def totalGames: Long = whiteWins + draws + blackWins

  def isEmpty = totalGames == 0
}
