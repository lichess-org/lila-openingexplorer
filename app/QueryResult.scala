package lila.openingexplorer

case class QueryResult(
    white: Long,
    draws: Long,
    black: Long,
    averageRating: Int,
    moves: List[(chess.MoveOrDrop, MoveStats)],
    recentGames: List[GameRef],
    topGames: List[GameRef]
) {

  def totalGames: Long = white + draws + black

  def isEmpty = totalGames == 0
}
