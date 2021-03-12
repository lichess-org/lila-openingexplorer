package lila.openingexplorer

import play.api.libs.json._

import chess.MoveOrDrop
import chess.opening.FullOpening

object JsonView {

  def masterEntry(fetchPgn: String => Option[String])(entry: QueryResult, opening: Option[FullOpening]) = {
    def refToJson(ref: GameRef) =
      fetchPgn(ref.gameId) flatMap GameInfo.parse map richGameRef(ref)
    baseEntry(entry, opening) ++ Json.obj(
      "topGames" -> entry.topGames.flatMap(refToJson)
    )
  }

  def lichessEntry(
      fetchInfo: String => Option[GameInfo]
  )(entry: QueryResult, opening: Option[FullOpening]) = {
    def refToJson(ref: GameRef) =
      fetchInfo(ref.gameId) map richGameRef(ref)
    baseEntry(entry, opening) ++ Json.obj(
      "recentGames" -> entry.recentGames.flatMap(refToJson),
      "topGames"    -> entry.topGames.flatMap(refToJson)
    )
  }

  def moveStats(moves: List[(MoveOrDrop, MoveStats)]) =
    JsArray {
      moves.map { case (move, stats) =>
        Json.obj(
          "uci"           -> move.fold(_.toUci, _.toUci).uci,
          "san"           -> move.fold(chess.format.pgn.Dumper(_), chess.format.pgn.Dumper(_)),
          "white"         -> stats.white,
          "draws"         -> stats.draws,
          "black"         -> stats.black,
          "averageRating" -> stats.averageRating
        )
      }
    }

  private def openingInfo(opening: FullOpening) =
    Json.obj(
      "eco"  -> opening.eco,
      "name" -> opening.name
    )

  private def baseEntry(entry: QueryResult, opening: Option[FullOpening]) =
    Json.obj(
      "white"         -> entry.white,
      "draws"         -> entry.draws,
      "black"         -> entry.black,
      "moves"         -> moveStats(entry.moves),
      "averageRating" -> entry.averageRating,
      "opening"       -> opening.map(openingInfo)
    )

  private def gameRef(ref: GameRef) =
    Json.obj(
      "id"     -> ref.gameId,
      "winner" -> ref.winner.fold("draw")(_.fold("white", "black")),
      "speed"  -> ref.speed.name
    )

  private def richGameRef(ref: GameRef)(info: GameInfo) =
    gameRef(ref) ++ Json.obj(
      "white" -> player(info.white),
      "black" -> player(info.black),
      "year"  -> info.year
    )

  private def player(p: GameInfo.Player) =
    Json.obj(
      "name"   -> p.name,
      "rating" -> p.rating
    )
}
