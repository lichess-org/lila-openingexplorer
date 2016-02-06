package controllers

import scala.concurrent.Future

import javax.inject.{Inject, Singleton}

import play.api.libs.json._
import play.api._
import play.api.mvc._
import play.api.inject.ApplicationLifecycle

import chess._
import chess.format.Forsyth
import chess.format.pgn.San
import chess.variant._

import lila.openingexplorer._

@Singleton
class WebApi @Inject() (
    protected val lifecycle: ApplicationLifecycle) extends Controller {

  val masterDb = new MasterDatabase()

  lifecycle.addStopHook { () =>
    Future.successful(masterDb.close)
  }

  private def gameRefToJson(ref: GameRef): JsValue = {
    Json.toJson(Map(
      "id"     -> Json.toJson(ref.gameId),
      "rating" -> Json.toJson(ref.averageRating),
      "winner" -> Json.toJson(ref.winner.map(_.fold("white", "black")).getOrElse("draw"))
    ))
  }

  private def moveEntriesToJson(children: List[(Move, SubEntry)], take: Int): JsArray = JsArray {
    children.filter(_._2.totalGames > 0).sortBy(-_._2.totalGames).take(take).map {
      case (move, entry) => Json.toJson(Map(
        "uci" -> Json.toJson(move.toUci.uci),
        "san" -> Json.toJson(chess.format.pgn.Dumper(move)),
        "total" -> Json.toJson(entry.totalGames),
        "white" -> Json.toJson(entry.whiteWins),
        "draws" -> Json.toJson(entry.draws),
        "black" -> Json.toJson(entry.blackWins),
        "averageRating" -> Json.toJson(entry.averageRating)
      ))
    }
  }

  def getMaster = Action { implicit req =>
    val fen = req.queryString get "fen" flatMap (_.headOption)
    val moves = req.queryString get "moves" flatMap (_.headOption) flatMap parseIntOption getOrElse 12

    fen.flatMap(Forsyth << _) match {
      case Some(situation) =>
        val entry = masterDb.probe(situation)

        Ok(Json.toJson(Map(
          "total" -> Json.toJson(entry.totalGames),
          "white" -> Json.toJson(entry.whiteWins),
          "draws" -> Json.toJson(entry.draws),
          "black" -> Json.toJson(entry.blackWins),
          "moves" -> moveEntriesToJson(masterDb.probeChildren(situation), moves),
          "averageRating" -> Json.toJson(entry.averageRating),
          "topGames" -> Json.toJson(entry.topGames.map(gameRefToJson))
        ))).withHeaders(
          "Access-Control-Allow-Origin" -> "*"
        )
      case None =>
        BadRequest("valid fen required")
    }
  }

  /* def get(name: String) = Action { implicit req =>
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
          "moves" -> moveEntriesToJson(db.probeChildren(category, situation), ratingGroups),
          "games" -> Json.toJson(entry.takeTopGames(Entry.maxGames).map(gameRefToJson))
        ))).withHeaders(
          "Access-Control-Allow-Origin" -> "*"
        )
      case None =>
        BadRequest("valid fen required")
    }
  } */

  def collectHashes(pgn: String, tags: List[chess.format.pgn.Tag]): Set[Array[Byte]] = {
    import chess.format.pgn.San
    def truncateMoves(moves: List[San]) = moves take 40

    chess.format.pgn.Reader.fullWithSans(pgn, truncateMoves, tags) match {
      case scalaz.Success(replay) =>
        (
          // the starting position
          List(masterDb.hash(replay.moves.last.fold(_.situationBefore, _.situationBefore))) ++
          // all others
          replay.moves.map(_.fold(_.situationAfter, _.situationAfter)).map(masterDb.hash(_))
        ).toSet
      case scalaz.Failure(e) =>
        Set.empty
    }
  }

  def putMaster = Action { implicit req =>
    val start = System.currentTimeMillis

    // todo: ensure this is safe
    val textBody = new String(req.body.asRaw.flatMap(_.asBytes()).getOrElse(Array.empty), "utf-8")
    val parsed = chess.format.pgn.Parser.full(textBody)

    parsed match {
      case scalaz.Success(game) =>
        val gameRef = GameRef.fromPgn(game)
        val hashes = collectHashes(textBody, game.tags)

        if (hashes.size >= 10) {
          hashes.foreach { h => masterDb.merge(h, gameRef) }

          val end = System.currentTimeMillis
          Ok("thanks. time taken: " ++ (end - start).toString ++ " ms")
        } else
          Ok("skipped: too few moves")

      case scalaz.Failure(e) =>
        BadRequest(e.toString)
    }
  }

}
