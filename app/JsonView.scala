package lila.openingexplorer

import play.api.libs.json._

import chess.MoveOrDrop

object JsonView {

  private type Children = List[(MoveOrDrop, SubEntry)]

  def masterEntry(fetchPgn: String => Option[String])(
    entry: SubEntry, children: List[(MoveOrDrop, SubEntry)]) = {
    def refToJson(ref: GameRef) =
      fetchPgn(ref.gameId) flatMap GameInfo.parse map richGameRef(ref)
    baseEntry(entry, children) ++ Json.obj(
      "recentGames" -> entry.recentGames.flatMap(refToJson),
      "topGames" -> entry.topGames.flatMap(refToJson))
  }

  def entry(entry: SubEntry, children: Children) =
    baseEntry(entry, children) ++ Json.obj(
      "recentGames" -> entry.recentGames.map(gameRef),
      "topGames" -> entry.topGames.map(gameRef))

  def moveEntries(elems: Children) = JsArray {
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

  private def baseEntry(entry: SubEntry, children: Children) = Json.obj(
    "total" -> entry.totalGames,
    "white" -> entry.whiteWins,
    "draws" -> entry.draws,
    "black" -> entry.blackWins,
    "moves" -> moveEntries(children),
    "averageRating" -> entry.averageRating)

  private def gameRef(ref: GameRef) = Json.obj(
    "id" -> ref.gameId,
    "rating" -> ref.averageRating,
    "winner" -> ref.winner.fold("draw")(_.fold("white", "black")))

  private def richGameRef(ref: GameRef)(info: GameInfo) = gameRef(ref) ++ Json.obj(
    "white" -> player(info.white),
    "black" -> player(info.black),
    "year" -> info.year)

  private def player(p: GameInfo.Player) = Json.obj(
    "name" -> p.name,
    "rating" -> p.rating)
}
