package controllers

import ornicar.scalalib.Validation
import scala.concurrent.Future
import scalaz.{ Success, Failure }

import javax.inject.{ Inject, Singleton }

import play.api._
import play.api.inject.ApplicationLifecycle
import play.api.mvc._

import chess.format.Forsyth

import lila.openingexplorer._

@Singleton
class WebApi @Inject() (
    protected val lifecycle: ApplicationLifecycle) extends Controller with Validation {

  val masterDb = new MasterDatabase()
  val lichessDb = new LichessDatabase()
  val importer = new Importer(masterDb, lichessDb)

  lifecycle.addStopHook { () =>
    Future.successful {
      masterDb.close
      lichessDb.closeAll
    }
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

  def getMasterPgn(gameId: String) = Action { implicit req =>
    masterDb.getPgn(gameId) match {
      case Some(pgn) => Ok(pgn)
      case None      => NotFound("game not found")
    }
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

  def putMaster = Action(parse.tolerantText) { implicit req =>
    importer.master(req.body) match {
      case (Success(_), ms)      => Ok(s"$ms ms")
      case (Failure(errors), ms) => BadRequest(errors.list.mkString)
    }
  }

  def putLichess(variantKey: String) = Action(parse.tolerantText) { implicit req =>
    chess.variant.Variant.byKey.get(variantKey).fold(BadRequest(s"Unknown variant $variantKey")) { variant =>
      importer.lichess(variant, req.body) match {
        case (_, ms) => Ok(s"$ms ms")
      }
    }
  }
}
