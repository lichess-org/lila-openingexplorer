package controllers

import scala.concurrent.Future
import scala.util.Random

import java.io.File

import javax.inject.{Inject, Singleton}

import play.api.libs.json._
import play.api._
import play.api.mvc._
import play.api.inject.ApplicationLifecycle

import fm.last.commons.kyoto.{KyotoDb}
import fm.last.commons.kyoto.factory.{KyotoDbBuilder, Mode}

import chess._
import chess.format.Forsyth
import chess.variant._
import chess.Hash

import lila.openingexplorer._

@Singleton
class WebApi @Inject() (
    protected val lifecycle: ApplicationLifecycle) extends Controller {

  val db = new KyotoDbBuilder("bullet.kct")
             .modes(Mode.READ_WRITE)
             .buckets(2 * 60 * 400000000L)  // twice the number of expected records
             .memoryMapSize(2147483648L)  // 2 gb
             .buildAndOpen()

  lifecycle.addStopHook { () =>
    Future.successful(db.close())
  }

  val hash = new Hash(32)  // 128 bit Zobrist hasher

  private def probe(h: Array[Byte]): Entry = {
    Option(db.get(h)) match {
      case Some(bytes) => Entry.unpack(bytes)
      case None        => Entry.empty
    }
  }

  private def probe(situation: Situation): Entry = probe(hash(situation))

  private def probeChildren(situation: Situation): Map[Move, Entry] = {
    situation.moves.values.flatten.map {
      case (move) => move -> probe(move.situationAfter)
    } toMap
  }

  private def merge(h: Array[Byte], entry: Entry) = {
    db.set(h, probe(h).combine(entry).pack)
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
          "total" -> Json.toJson(entry.sumGames(ratingGroups)),
          "white" -> Json.toJson(entry.sumWhiteWins(ratingGroups)),
          "draws" -> Json.toJson(entry.sumDraws(ratingGroups)),
          "black" -> Json.toJson(entry.sumBlackWins(ratingGroups))
        ))
    }.toMap)
  }

  def index() = Action { implicit req =>
    val fen = req.queryString get "fen" flatMap (_.headOption)

    val ratingGroups = RatingGroup.range(
      req.queryString get "minRating" flatMap (_.headOption) flatMap parseIntOption,
      req.queryString get "maxRating" flatMap (_.headOption) flatMap parseIntOption
    )

    fen.flatMap(Forsyth << _) match {
      case Some(situation) =>
        val entry = probe(situation)

        Ok(Json.toJson(Map(
          "total" -> Json.toJson(entry.sumGames(ratingGroups)),
          "white" -> Json.toJson(entry.sumWhiteWins(ratingGroups)),
          "draws" -> Json.toJson(entry.sumDraws(ratingGroups)),
          "black" -> Json.toJson(entry.sumBlackWins(ratingGroups)),
          "moves" -> moveMapToJson(probeChildren(situation), ratingGroups),
          "games" -> Json.toJson(entry.takeTopGames(Entry.maxGames).map(gameRefToJson))
        ))).withHeaders(
          "Access-Control-Allow-Origin" -> "*"
        )
      case None =>
        BadRequest("valid fen required")
    }
  }

  def winner(game: chess.format.pgn.ParsedPgn) = {
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
        val gameRef = new GameRef(Random.alphanumeric.take(8).mkString, 3000, winner(game))
        val entry = Entry.fromGameRef(gameRef)

        chess.format.pgn.Reader.fullWithSans(textBody, identity, game.tags) match {
          case scalaz.Success(replay) if replay.moves.size >= 2 =>
            // todo: use lichess game ids, not fics
            // todo: should we index unrated games?
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

            val entry = Entry.fromGameRef(gameRef)

            val hashes = (
              // the starting position
              List(hash(replay.moves.last.fold(_.situationBefore, _.situationBefore))) ++
              // all others
              replay.moves.map(_.fold(_.situationAfter, _.situationAfter)).map(hash(_))
            ).toSet

            hashes.foreach { h => merge(h, entry) }

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
