package controllers

import com.github.blemale.scaffeine.{ LoadingCache, Scaffeine }
import ornicar.scalalib.Validation
import scalaz.{ Failure, Success }

import javax.inject.{ Inject, Singleton }

import play.api.libs.json._
import play.api.mvc._

import chess.format.Forsyth

import lila.openingexplorer._

@Singleton
class WebApi @Inject() (
    cc: ControllerComponents,
    config: Config,
    masterDb: MasterDatabase,
    lichessDb: LichessDatabase,
    pgnDb: PgnDatabase,
    gameInfoDb: GameInfoDatabase,
    importer: Importer
) extends AbstractController(cc)
    with Validation
    with play.api.i18n.I18nSupport {

  private val cacheConfig = config.explorer.cache

  private val masterCache: LoadingCache[Forms.master.Data, String] = Scaffeine()
    .expireAfterAccess(cacheConfig.ttl)
    .maximumSize(10000)
    .build(fetchMaster)

  private def fetchMaster(data: Forms.master.Data): String =
    (Forsyth << data.fen).fold("") { situation =>
      val result = masterDb.query(situation, data.movesOrDefault, data.topGamesOrDefault)
      Json stringify JsonView.masterEntry(pgnDb.get)(result)
    }

  def getMaster = Action { implicit req =>
    CORS {
      Forms.master.form.bindFromRequest.fold(
        err => BadRequest(err.errorsAsJson),
        data =>
          (Forsyth << data.fen) match {
            case Some(situation) =>
              JsonResult {
                fenMoveNumber(data.fen).fold(fetchMaster _) { moveNumber =>
                  if (moveNumber > cacheConfig.maxMoves) fetchMaster _
                  else masterCache.get _
                }(data)
              }
            case None => BadRequest("valid fen required")
          }
      )
    }
  }

  def deleteMaster(gameId: String) = Action {
    if (importer.deleteMaster(gameId))
      Status(204)
    else
      NotFound("game not found")
  }

  def getMasterPgn(gameId: String) = Action {
    pgnDb.get(gameId) match {
      case Some(pgn) => Ok(pgn)
      case None      => NotFound("game not found")
    }
  }

  private val lichessCache: LoadingCache[Forms.lichess.Data, String] = Scaffeine()
    .expireAfterAccess(cacheConfig.ttl)
    .maximumSize(10000)
    .build(fetchLichess)

  private def situationOf(data: Forms.lichess.Data) =
    (Forsyth << data.fen) map (_ withVariant data.actualVariant)

  private def fetchLichess(data: Forms.lichess.Data): String =
    situationOf(data).fold("") { situation =>
      val request = LichessDatabase.Request(
        data.speedGroups,
        data.ratingGroups,
        data.topGamesOrDefault,
        data.recentGamesOrDefault,
        data.movesOrDefault
      )

      val entry = lichessDb.query(situation, request)
      Json stringify JsonView.lichessEntry(gameInfoDb.get)(entry)
    }

  def getLichess = Action { implicit req =>
    CORS {
      Forms.lichess.form.bindFromRequest.fold(
        err => BadRequest(err.errorsAsJson),
        data =>
          situationOf(data) match {
            case Some(situation) =>
              JsonResult {
                fenMoveNumber(data.fen).fold(fetchLichess _) { moveNumber =>
                  if (moveNumber > cacheConfig.maxMoves || !data.fullHouse) fetchLichess _
                  lichessCache.get _
                }(data)
              }
            case None => BadRequest("valid fen required")
          }
      )
    }
  }

  def getStats = Action {
    CORS {
      JsonResult {
        Json stringify Json.obj(
          "master" -> Json.obj(
            "games"           -> pgnDb.count,
            "uniquePositions" -> masterDb.uniquePositions
          ),
          "lichess" -> Json.toJson(
            lichessDb.variants
              .map({
                case variant =>
                  variant.key -> Json.obj(
                    "games"           -> lichessDb.totalGames(variant),
                    "uniquePositions" -> lichessDb.uniquePositions(variant)
                  )
              })
              .toMap
          )
        )
      }
    }
  }

  def putMaster = Action(parse.tolerantText) { implicit req =>
    importer.master(req.body) match {
      case (Success(_), ms)      => Ok(s"$ms ms")
      case (Failure(errors), ms) => BadRequest(errors.list.toList.mkString)
    }
  }

  def putLichess = Action(parse.tolerantText) { implicit req =>
    importer.lichess(req.body) match {
      case (_, ms) => Ok(s"$ms ms")
    }
  }

  def CORS(res: Result) =
    if (config.explorer.corsHeader) res.withHeaders("Access-Control-Allow-Origin" -> "*")
    else res

  def JsonResult(json: String) = Ok(json).as("application/json; charset=utf-8")

  private def fenMoveNumber(fen: String) = fen split ' ' lift 5 flatMap (_.toIntOption)
}
