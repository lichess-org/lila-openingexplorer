package controllers

import scala.concurrent.Future

import javax.inject.{ Inject, Singleton }

import play.api._
import play.api.inject.ApplicationLifecycle
import play.api.mvc._

import chess.format.Forsyth
import chess.format.pgn.San
import chess.PositionHash
import chess.variant._

import lila.openingexplorer._

@Singleton
class WebApi @Inject() (
    protected val lifecycle: ApplicationLifecycle) extends Controller {

  val masterDb = new MasterDatabase()

  lifecycle.addStopHook { () =>
    Future.successful(masterDb.close)
  }

  def getMaster = Action { implicit req =>
    Forms.master.form.bindFromRequest.fold(
      err => BadRequest(err.toString),
      data => {
        (Forsyth << data.fen) match {
          case Some(situation) =>
            val entry = masterDb.probe(situation)
            val children = masterDb.probeChildren(situation)
              .filter(_._2.totalGames > 0)
              .sortBy(-_._2.totalGames)
              .take(data.movesOrDefault)
            Ok(JsonView.entry(entry, children)).withHeaders(
              "Access-Control-Allow-Origin" -> "*"
            )
          case None =>
            BadRequest("valid fen required")
        }
      })
  }

  def getLichess = Action { implicit req =>
    Forms.lichess.form.bindFromRequest.fold(
      err => BadRequest(err.toString),
      data => {
        println(data)
        Ok
      })
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

  private def collectHashes(parsed: chess.format.pgn.ParsedPgn): Set[PositionHash] = {
    import chess.format.pgn.San
    def truncateMoves(moves: List[San]) = moves take 50

    chess.format.pgn.Reader.fullWithSans(parsed, truncateMoves _) match {
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

  def putMaster = Action(parse.tolerantText) { implicit req =>
    val start = System.currentTimeMillis

    chess.format.pgn.Parser.full(req.body) match {
      case scalaz.Success(parsed) => GameRef.fromPgn(parsed) match {
        case Left(error) => Ok(s"skipped: $error")
        case Right(gameRef) =>
          masterDb.mergeAll(collectHashes(parsed), gameRef)

          val end = System.currentTimeMillis
          Ok(s"thanks. time taken: ${end - start} ms")
      }

      case scalaz.Failure(e) =>
        BadRequest(e.toString)
    }
  }

}
