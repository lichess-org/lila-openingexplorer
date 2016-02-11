package controllers

import ornicar.scalalib.Validation
import scala.concurrent.Future
import scalaz.{ Success, Failure }

import javax.inject.{ Inject, Singleton }

import play.api._
import play.api.i18n.Messages.Implicits._
import play.api.inject.ApplicationLifecycle
import play.api.mvc._
import play.api.Play.current

import chess.format.Forsyth

import lila.openingexplorer._

@Singleton
class WebApi @Inject() (
    protected val lifecycle: ApplicationLifecycle) extends Controller with Validation {

  val masterDb = new MasterDatabase()
  val lichessDb = new LichessDatabase()
  val pgnDb = new PgnDatabase()
  val gameInfoDb = new GameInfoDatabase()

  val importer = new Importer(masterDb, lichessDb, pgnDb, gameInfoDb)

  lifecycle.addStopHook { () =>
    Future.successful {
      masterDb.close
      lichessDb.closeAll
      pgnDb.close
    }
  }

  def getMaster = Action { implicit req =>
    CORS {
      Forms.master.form.bindFromRequest.fold(
        err => BadRequest(err.errorsAsJson),
        data => (Forsyth << data.fen) match {
          case Some(situation) =>
            val entry = masterDb.probe(situation)
            val children = masterDb.probeChildren(situation)
              .filter(_._2.totalGames > 0)
              .sortBy(-_._2.totalGames)
              .take(data.movesOrDefault)
            Ok(JsonView.masterEntry(pgnDb.get)(entry, children))
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
          case Some(situation) =>
            val request = LichessDatabase.Request(data.speedGroups, data.ratingGroups)
            val entry = lichessDb.probe(situation, request)
            val children = lichessDb.probeChildren(situation, request)
              .filter(_._2.totalGames > 0)
              .sortBy(-_._2.totalGames)
              .take(data.movesOrDefault)
            Ok(JsonView.entry(entry, children))
          case None => BadRequest("valid fen required")
        }
      )
    }
  }

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
}
