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

import lila.openingexplorer.Entry

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

  def index() = Action { implicit req =>
    val fen = req.queryString get "fen" flatMap (_.headOption)
    val situation = fen.flatMap(Forsyth << _)

    situation.map(probe(_)) match {
      case Some(entry) => Ok(entry.totalGames.toString)
      case None        => BadRequest("valid fen required")
    }
  }

}
