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

@Singleton
class WebApi @Inject() (
    val cacheApi: CacheApi,
    val lifecycle: ApplicationLifecycle) extends Controller with Validation {

  val masterDb = new MasterDatabase()
  val lichessDb = new LichessDatabase()
  val pgnDb = new PgnDatabase()
  val gameInfoDb = new GameInfoDatabase()

  val importer = new Importer(masterDb, lichessDb, pgnDb, gameInfoDb)
  val cache = new Cache(cacheApi)

  lifecycle.addStopHook { () =>
    Future.successful {
      masterDb.close
      lichessDb.closeAll
      pgnDb.close
      gameInfoDb.close
    }
  }

  def getMaster = Action { implicit req =>
    CORS {
      Forms.master.form.bindFromRequest.fold(
        err => BadRequest(err.errorsAsJson),
        data => (Forsyth << data.fen) match {
          case Some(situation) => JsonResult {
            cache.master(data.fen) {
              val entry = masterDb.query(situation)
              val children = curate(masterDb.queryChildren(situation), data.movesOrDefault)
              Json stringify JsonView.masterEntry(pgnDb.get)(entry, children, data.fen)
            }
          }
          case None => BadRequest("valid fen required")
        }
      )
    }
  }

  def getMasterPgn(gameId: String) = Action { implicit req =>
    pgnDb.get(gameId) match {
      case Some(pgn) => Ok(pgn)
      case None      => NotFound("game not found")
    }
  }

  def getLichess = Action { implicit req =>
    CORS {
      Forms.lichess.form.bindFromRequest.fold(
        err => BadRequest(err.errorsAsJson),
        data => (Forsyth << data.fen) map (_ withVariant data.actualVariant) match {
          case Some(situation) => JsonResult {
            cache.lichess(data) {
              val request = LichessDatabase.Request(data.speedGroups, data.ratingGroups)
              val entry = lichessDb.query(situation, request)
              val children = curate(lichessDb.queryChildren(situation, request), data.movesOrDefault)
              Json stringify JsonView.lichessEntry(gameInfoDb.get)(entry, children, data.fen)
            }
          }
          case None => BadRequest("valid fen required")
        }
      )
    }
  }

  private def curate(children: Children, max: Int) =
    children.filterNot(_._2.isEmpty).sortBy(-_._2.totalGames).take(max)

  def putMaster = Action(parse.tolerantText) { implicit req =>
    importer.master(req.body) match {
      case (Success(_), ms)      => Ok(s"$ms ms")
      case (Failure(errors), ms) => BadRequest(errors.list.mkString)
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
      case Some(callback) => Ok(s"$callback($json)").as("application/javascript")
      case None =>           Ok(json).as("application/json")
    }
}
