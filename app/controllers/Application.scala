package controllers

import java.io.File

import play.api.libs.json._
import play.api._
import play.api.mvc._

import fm.last.commons.kyoto.{KyotoDb}
import fm.last.commons.kyoto.factory.{KyotoDbBuilder, Mode}

import chess._
import chess.format.Forsyth
import chess.variant._

class Application extends Controller {

  def index(variant: String, rating: String) = Action { implicit ctx =>
    val file = new File("bullet.kct")
    file.createNewFile()

    val db = new KyotoDbBuilder(file)
                .modes(Mode.CREATE, Mode.READ_WRITE)
                .buildAndOpen()

    db.set("hello", "world")

    val position = "8/3k4/2q5/8/8/K1B5/8/8 w - -"

    val situation = Forsyth << position

    val moves = situation match {
      case Some(s) => s.moves.values.flatten.map {
        case (move) => Json.toJson(move.toString)
      }
    }

    Ok(db.get("hello"))
  }

}
