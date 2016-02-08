package lila.openingexplorer

import play.api.libs.json._

import chess.Move

object JsonView {

  def entry(entry: SubEntry, children: List[(Move, SubEntry)]) = Json.obj(
    "total" -> entry.totalGames,
    "white" -> entry.whiteWins,
    "draws" -> entry.draws,
    "black" -> entry.blackWins,
    "moves" -> moveEntries(children),
    "averageRating" -> entry.averageRating,
    "recentGames" -> entry.recentGames.map(gameRef),
    "topGames" -> entry.topGames.map(gameRef))

  def moveEntries(elems: List[(Move, SubEntry)]) = JsArray {
    elems.map {
      case (move, entry) => Json.obj(
        "uci" -> move.toUci.uci,
        "san" -> chess.format.pgn.Dumper(move),
        "total" -> entry.totalGames,
        "white" -> entry.whiteWins,
        "draws" -> entry.draws,
        "black" -> entry.blackWins,
        "averageRating" -> entry.averageRating)
    }
  }

  def gameRef(ref: GameRef) = Json.obj(
    "id" -> ref.gameId,
    "rating" -> ref.averageRating,
    "winner" -> ref.winner.fold("draw")(_.fold("white", "black")))
}
