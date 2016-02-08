package lila.openingexplorer

import play.api.libs.json._

import chess.MoveOrDrop

object JsonView {

  def entry(entry: SubEntry, children: List[(MoveOrDrop, SubEntry)]) = Json.obj(
    "total" -> entry.totalGames,
    "white" -> entry.whiteWins,
    "draws" -> entry.draws,
    "black" -> entry.blackWins,
    "moves" -> moveEntries(children),
    "averageRating" -> entry.averageRating,
    "recentGames" -> entry.recentGames.map(gameRef),
    "topGames" -> entry.topGames.map(gameRef))

  def moveEntries(elems: List[(MoveOrDrop, SubEntry)]) = JsArray {
    elems.map {
      case (move, entry) => Json.obj(
        "uci" -> move.fold(_.toUci, _.toUci).uci,
        "san" -> move.fold(chess.format.pgn.Dumper(_), chess.format.pgn.Dumper(_)),
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
