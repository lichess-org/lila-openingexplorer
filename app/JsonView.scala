package lila.openingexplorer

import play.api.libs.json._

import chess.MoveOrDrop

object JsonView {

  def masterEntry(fetchPgn: String => Option[String])(result: MasterQueryResult) = {
    def refToJson(ref: GameRef) =
      fetchPgn(ref.gameId) flatMap GameInfo.parse map richGameRef(ref)

    Json.obj(
      "white" -> result.white,
      "draws" -> result.draws,
      "black" -> result.black,
      "averageRating" -> result.averageRating,
      "moves" -> moveStats(result.moves),
      "topGames" -> result.topGames.flatMap(refToJson))
  }

  def lichessEntry(fetchInfo: String => Option[GameInfo])(entry: QueryResult, children: Children, fen: String) = {
    def refToJson(ref: GameRef) =
      fetchInfo(ref.gameId) map richGameRef(ref)
    baseEntry(entry, children, fen) ++ Json.obj(
      "recentGames" -> entry.recentGames.flatMap(refToJson),
      "topGames" -> entry.topGames.flatMap(refToJson))
  }

  def moveStats(moves: List[(MoveOrDrop, MoveStats)]) = JsArray {
    moves.map {
      case (move, stats) => Json.obj(
        "uci" -> move.fold(_.toUci, _.toUci).uci,
        "san" -> move.fold(chess.format.pgn.Dumper(_), chess.format.pgn.Dumper(_)),
        "white" -> stats.white,
        "draws" -> stats.draws,
        "black" -> stats.black,
        "averageRating" -> stats.averageRating)
    }
  }

  def moveEntries(elems: Children) = JsArray {
    elems.map {
      case (move, entry) => Json.obj(
        "uci" -> move.fold(_.toUci, _.toUci).uci,
        "san" -> move.fold(chess.format.pgn.Dumper(_), chess.format.pgn.Dumper(_)),
        "white" -> entry.whiteWins,
        "draws" -> entry.draws,
        "black" -> entry.blackWins,
        "averageRating" -> entry.averageRating)
    }
  }

  private def baseEntry(entry: QueryResult, children: Children, fen: String) = Json.obj(
    "fen" -> fen,
    "white" -> entry.whiteWins,
    "draws" -> entry.draws,
    "black" -> entry.blackWins,
    "moves" -> moveEntries(children),
    "averageRating" -> entry.averageRating)

  private def gameRef(ref: GameRef) = Json.obj(
    "id" -> ref.gameId,
    "winner" -> ref.winner.fold("draw")(_.fold("white", "black")))

  private def richGameRef(ref: GameRef)(info: GameInfo) = gameRef(ref) ++ Json.obj(
    "white" -> player(info.white),
    "black" -> player(info.black),
    "year" -> info.year)

  private def player(p: GameInfo.Player) = Json.obj(
    "name" -> p.name,
    "rating" -> p.rating)
}
