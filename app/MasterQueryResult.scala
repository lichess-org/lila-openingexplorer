package lila.openingexplorer

case class MasterQueryResult(
    white: Long,
    draws: Long,
    black: Long,
    averageRating: Int,
    moves: List[(chess.MoveOrDrop, MoveStats)],
    topGames: List[GameRef]) {

  def totalGames: Long = white + draws + black

  def isEmpty = totalGames == 0
}
