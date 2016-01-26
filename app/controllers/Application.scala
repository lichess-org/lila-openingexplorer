package controllers

import play.api.libs.json._
import play.api._
import play.api.mvc._

import chess._
import chess.format.Forsyth
import chess.variant._

class Application extends Controller {

  def index(variant: String, rating: String) = Action { implicit ctx =>
    val db = new kyotocabinet.DB()

    val position = "8/3k4/2q5/8/8/K1B5/8/8 w - -"

    val situation = Forsyth << position

    val moves = situation match {
      case Some(s) => s.moves.values.flatten.map {
        case (move) => Json.toJson(move.toString)
      }
    }

    Ok(variant)
  }

}
