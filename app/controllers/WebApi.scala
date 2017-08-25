package controllers

import ornicar.scalalib.Validation
import scala.concurrent.Future
import scalaz.{ Success, Failure }

import javax.inject.{ Inject, Singleton }

import play.api._
import play.api.cache.CacheApi
import play.api.i18n.Messages.Implicits._
import play.api.inject.ApplicationLifecycle
import play.api.libs.json.Json
import play.api.mvc._
import play.api.Play.current

import chess.format.Forsyth

import lila.openingexplorer._

class WebApi @Inject() (
    cc: ControllerComponents,
    masterDb: MasterDatabase,
    lichessDb: LichessDatabase,
    pgnDb: PgnDatabase,
    gameInfoDb: GameInfoDatabase,
    importer: Importer,
    cache: Cache
) extends AbstractController(cc) with Validation with play.api.i18n.I18nSupport {

  def getMaster = Action { implicit req =>
    CORS {
      Forms.master.form.bindFromRequest.fold(
        err => BadRequest(err.errorsAsJson),
        data => (Forsyth << data.fen) match {
          case Some(situation) => JsonResult {
            cache.master(data) {
              val result = masterDb.query(situation, data.movesOrDefault, data.topGamesOrDefault)
              Json stringify JsonView.masterEntry(pgnDb.get)(result)
            }
          }
          case None => BadRequest("valid fen required")
        }
      )
    }
  }

  def deleteMaster(gameId: String) = Action { implicit req =>
    if (importer.deleteMaster(gameId))
      Status(204)
    else
      NotFound("game not found")
  }

  def getMasterPgn(gameId: String) = Action { implicit req =>
    pgnDb.get(gameId) match {
      case Some(pgn) => Ok(pgn)
      case None => NotFound("game not found")
    }
  }

  def getLichess = Action { implicit req =>
    CORS {
      Forms.lichess.form.bindFromRequest.fold(
        err => BadRequest(err.errorsAsJson),
        data => (Forsyth << data.fen) map (_ withVariant data.actualVariant) match {
          case Some(situation) => JsonResult {
            cache.lichess(data) {
              val request = LichessDatabase.Request(
                data.speedGroups, data.ratingGroups,
                data.topGamesOrDefault, data.recentGamesOrDefault,
                data.movesOrDefault
              )

              val entry = lichessDb.query(situation, request)
              Json stringify JsonView.lichessEntry(gameInfoDb.get)(entry)
            }
          }
          case None => BadRequest("valid fen required")
        }
      )
    }
  }

  def getStats = Action { implicit req =>
    CORS {
      JsonResult {
        cache.stat {
          Json stringify Json.obj(
            "master" -> Json.obj(
              "games" -> pgnDb.count,
              "uniquePositions" -> masterDb.uniquePositions
            ),
            "lichess" -> Json.toJson(lichessDb.variants.map({
              case variant =>
                variant.key -> Json.obj(
                  "games" -> lichessDb.totalGames(variant),
                  "uniquePositions" -> lichessDb.uniquePositions(variant)
                )
            }).toMap)
          )
        }
      }
    }
  }

  def putMaster = Action(parse.tolerantText) { implicit req =>
    importer.master(req.body) match {
      case (Success(_), ms) => Ok(s"$ms ms")
      case (Failure(errors), ms) => BadRequest(errors.list.toList.mkString)
    }
  }

  def putLichess = Action(parse.tolerantText) { implicit req =>
    importer.lichess(req.body) match {
      case (_, ms) => Ok(s"$ms ms")
    }
  }

  def CORS(res: Result) =
    if (Config.explorer.corsHeader) res.withHeaders("Access-Control-Allow-Origin" -> "*")
    else res

  def JsonResult(json: String)(implicit req: RequestHeader) =
    req.queryString.get("callback").flatMap(_.headOption) match {
      case Some(callback) => Ok(s"$callback($json)").as("application/javascript; charset=utf-8")
      case None => Ok(json).as("application/json; charset=utf-8")
    }
}
