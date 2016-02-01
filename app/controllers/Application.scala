package controllers

import scala.concurrent.Future

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

import lila.openingexplorer.{Entry, GameRef}

@Singleton
class Application @Inject() (
    protected val lifecycle: ApplicationLifecycle) extends Controller {

  val db = new KyotoDbBuilder("bullet.kct")
             .modes(Mode.CREATE, Mode.READ_WRITE)
             .buildAndOpen()

  lifecycle.addStopHook { () =>
    Future.successful(db.close())
  }

  val hash = new Hash(32)  // 128 bit Zobrist hasher

  private def probe(situation: Situation): Entry = {
    val query = Option(db.get(hash(situation)))

    query match {
      case Some(bytes) => Entry.unpack(bytes)
      case None        => Entry.empty
    }
  }

  private def merge(situation: Situation, entry: Entry) = {
    val merged = probe(situation).combine(entry)
    db.set(hash(situation), merged.pack)
  }

  def index() = Action { implicit req =>
    val fen = req.queryString get "fen" flatMap (_.headOption)
    val situation = fen.flatMap(Forsyth << _)

    situation.map(probe(_)) match {
      case Some(entry) => Ok(entry.totalGames.toString)
      case None        => BadRequest("valid fen required")
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

    val textBody = new String(req.body.asRaw.flatMap(_.asBytes()).getOrElse(Array.empty), "UTF-8")
    println(textBody)
    println("hello")
    val parsed = chess.format.pgn.Parser.full(textBody)

    parsed match {
      case scalaz.Success(game) =>
        val gameRef = new GameRef("zzzzzzzz", 3000, winner(game))
        val entry = Entry.fromGameRef(gameRef)

        Ok("thanks. time taken: " ++ (System.currentTimeMillis - start).toString ++ " ms")
      case scalaz.Failure(e) =>
        BadRequest(e.toString)
    }
  }

}
