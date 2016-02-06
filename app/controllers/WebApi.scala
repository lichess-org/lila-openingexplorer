package controllers

import scala.concurrent.Future
import scala.util.Random

import javax.inject.{Inject, Singleton}

import play.api.libs.json._
import play.api._
import play.api.mvc._
import play.api.inject.ApplicationLifecycle

import chess._
import chess.format.Forsyth
import chess.variant._

import lila.openingexplorer._

@Singleton
class WebApi @Inject() (
    protected val lifecycle: ApplicationLifecycle) extends Controller {

  val db = new Database()

  lifecycle.addStopHook { () =>
    Future.successful(db.closeAll)
  }

  private def gameRefToJson(ref: GameRef): JsValue = {
    Json.toJson(Map(
      "id"     -> Json.toJson(ref.gameId),
      "rating" -> Json.toJson(ref.rating),
      "winner" -> Json.toJson(ref.winner.map(_.fold("white", "black")).getOrElse("draw"))
    ))
  }

  private def moveMapToJson(
      children: Map[Move, Entry],
      ratingGroups: List[RatingGroup]): JsValue = {
    Json.toJson(children.map {
      case (move, entry) =>
        move.toUci.uci -> Json.toJson(Map(
          "uci" -> Json.toJson(move.toUci.uci),
          "san" -> Json.toJson(chess.format.pgn.Dumper(move)),
          "total" -> Json.toJson(entry.sumGames(ratingGroups)),
          "white" -> Json.toJson(entry.sumWhiteWins(ratingGroups)),
          "draws" -> Json.toJson(entry.sumDraws(ratingGroups)),
          "black" -> Json.toJson(entry.sumBlackWins(ratingGroups))
        ))
    }.toMap)
  }

  def get(name: String) = Action { implicit req =>
    Category.find(name) match {
      case Some(category) => getCategory(category)
      case None           => NotFound("category not found")
    }
  }

  def getCategory(category: Category)(implicit req: RequestHeader) = {
    val fen = req.queryString get "fen" flatMap (_.headOption)

    val ratingGroups = RatingGroup.range(
      req.queryString get "minRating" flatMap (_.headOption) flatMap parseIntOption,
      req.queryString get "maxRating" flatMap (_.headOption) flatMap parseIntOption
    )

    fen.flatMap(Forsyth << _).map(_.withVariant(category.variant)) match {
      case Some(situation) =>
        val entry = db.probe(category, situation)

        Ok(Json.toJson(Map(
          "total" -> Json.toJson(entry.sumGames(ratingGroups)),
          "white" -> Json.toJson(entry.sumWhiteWins(ratingGroups)),
          "draws" -> Json.toJson(entry.sumDraws(ratingGroups)),
          "black" -> Json.toJson(entry.sumBlackWins(ratingGroups)),
          "moves" -> moveMapToJson(db.probeChildren(category, situation), ratingGroups),
          "games" -> Json.toJson(entry.takeTopGames(Entry.maxGames).map(gameRefToJson))
        ))).withHeaders(
          "Access-Control-Allow-Origin" -> "*"
        )
      case None =>
        BadRequest("valid fen required")
    }
  }

  private def winner(game: chess.format.pgn.ParsedPgn) = {
    game.tag("Result") match {
      case Some("1-0") => Some(Color.White)
      case Some("0-1") => Some(Color.Black)
      case _           => None
    }
  }

  def put() = Action { implicit req =>
    val start = System.currentTimeMillis

    // todo: ensure this is safe
    val textBody = new String(req.body.asRaw.flatMap(_.asBytes()).getOrElse(Array.empty), "UTF-8")
    val parsed = chess.format.pgn.Parser.full(textBody)

    parsed match {
      case scalaz.Success(game) =>
        chess.format.pgn.Reader.fullWithSans(textBody, identity, game.tags) match {
          case scalaz.Success(replay) if replay.moves.size >= 2 =>
            // todo: use lichess game ids, not fics
            val gameRef = new GameRef(
              game.tag("FICSGamesDBGameNo")
                .flatMap(parseLongOption)
                .map(GameRef.unpackGameId)
                .getOrElse(Random.alphanumeric.take(8).mkString),
              math.min(
                game.tag("WhiteElo").flatMap(parseIntOption).getOrElse(0),
                game.tag("BlackElo").flatMap(parseIntOption).getOrElse(0)
              ),
              winner(game)
            )

            val hashes = (
              // the starting position
              List(db.hash(replay.moves.last.fold(_.situationBefore, _.situationBefore))) ++
              // all others
              replay.moves.map(_.fold(_.situationAfter, _.situationAfter)).map(db.hash(_))
            ).toSet

            hashes.foreach { h => db.merge(Category.Bullet, h, gameRef) }

            val end = System.currentTimeMillis
            Ok("thanks. time taken: " ++ (end - start).toString ++ " ms")

          case scalaz.Success(game) =>
            Ok("skipped: too few moves")

          case scalaz.Failure(e) =>
            BadRequest(e.toString)
        }

      case scalaz.Failure(e) =>
        BadRequest(e.toString)
    }
  }

}
