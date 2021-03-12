package controllers

import com.github.blemale.scaffeine.{ LoadingCache, Scaffeine }

import javax.inject.{ Inject, Singleton }

import cats.data.Validated

import play.api.libs.json._
import play.api.mvc._

import chess.Situation
import chess.variant.Variant
import chess.format.{ FEN, Forsyth, Uci }
import chess.opening.{ FullOpening, FullOpeningDB }

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
    with play.api.i18n.I18nSupport {

  private def findOpening(sit: Situation): Option[FullOpening] =
    if (Variant.openingSensibleVariants(sit.board.variant)) FullOpeningDB.findByFen(Forsyth >> sit)
    else None

  private def play(
      situation: Option[Situation],
      line: Option[String]
  ): Option[(Situation, Option[FullOpening])] = {
    val moves = line.filter(_ != "").map(_.split(",")).getOrElse(Array())
    moves.foldLeft(situation.map(sit => (sit, findOpening(sit)))) {
      case (Some((sit, opening)), uci) =>
        Uci(uci).flatMap(m =>
          m(sit) match {
            case Validated.Valid(Left(move)) =>
              Some((move.situationAfter, findOpening(move.situationAfter) orElse opening))
            case Validated.Valid(Right(drop)) =>
              Some((drop.situationAfter, opening))
            case Validated.Invalid(_) => None
          }
        )
      case (None, _) => None
    }
  }

  private def situationOf(data: Forms.master.Data) =
    play((Forsyth << FEN(data.fen)), data.play)

  private def situationOf(data: Forms.lichess.Data) =
    play((Forsyth << FEN(data.fen)) map (_ withVariant data.actualVariant), data.play)

  private val cacheConfig = config.explorer.cache

  private val masterCache: LoadingCache[Forms.master.Data, String] = Scaffeine()
    .expireAfterWrite(cacheConfig.ttl)
    .maximumSize(10000)
    .build(fetchMaster)

  private def fetchMaster(data: Forms.master.Data): String =
    situationOf(data).fold("") {
      case (situation, opening) =>
        val result = masterDb.query(situation, data.movesOrDefault, data.topGamesOrDefault)
        Json stringify JsonView.masterEntry(pgnDb.get)(result, opening)
    }

  def getMaster =
    Action { implicit req =>
      CORS {
        Forms.master.form
          .bindFromRequest()
          .fold(
            err => BadRequest(err.errorsAsJson),
            data =>
              situationOf(data) match {
                case Some(situation) =>
                  JsonResult {
                    fenMoveNumber(data.fen).fold(fetchMaster _) { moveNumber =>
                      if (moveNumber + data.play.size / 2 > cacheConfig.maxMoves) fetchMaster _
                      else masterCache.get _
                    }(data)
                  }
                case None => BadRequest("valid position required")
              }
          )
      }
    }

  def deleteMaster(gameId: String) =
    Action {
      if (importer.deleteMaster(gameId))
        Status(204)
      else
        NotFound("game not found")
    }

  def getMasterPgn(gameId: String) =
    Action {
      pgnDb.get(gameId) match {
        case Some(pgn) => Ok(pgn)
        case None      => NotFound("game not found")
      }
    }

  private val lichessCache: LoadingCache[Forms.lichess.Data, String] = Scaffeine()
    .expireAfterWrite(cacheConfig.ttl)
    .maximumSize(10000)
    .build(fetchLichess)

  private def fetchLichess(data: Forms.lichess.Data): String =
    situationOf(data).fold("") {
      case (situation, opening) =>
        val request = LichessDatabase.Request(
          data.speedGroups,
          data.ratingGroups,
          data.topGamesOrDefault,
          data.recentGamesOrDefault,
          data.movesOrDefault
        )

        val entry = lichessDb.query(situation, request)
        Json stringify JsonView.lichessEntry(gameInfoDb.get)(entry, opening)
    }

  def getLichess =
    Action { implicit req =>
      CORS {
        Forms.lichess.form
          .bindFromRequest()
          .fold(
            err => BadRequest(err.errorsAsJson),
            data =>
              situationOf(data) match {
                case Some(situation) =>
                  JsonResult {
                    fenMoveNumber(data.fen).fold(fetchLichess _) { moveNumber =>
                      if (moveNumber + data.play.size / 2 > cacheConfig.maxMoves || !data.fullHouse)
                        fetchLichess _
                      else lichessCache.get _
                    }(data)
                  }
                case None => BadRequest("valid position required")
              }
          )
      }
    }

  def getStats =
    Action {
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

  def putMaster =
    Action(parse.tolerantText) { implicit req =>
      importer.master(req.body) match {
        case (Validated.Valid(_), ms)       => Ok(s"$ms ms")
        case (Validated.Invalid(error), ms) => BadRequest(error)
      }
    }

  def putLichess =
    Action(parse.tolerantText) { implicit req =>
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
