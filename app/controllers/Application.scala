package controllers

import play.api.libs.json._
import play.api._
import play.api.mvc._

class Application extends Controller {

  def index = Action { req =>
    Ok(Json.obj(
      "hello" -> "world"
    )) as JSON
  }

}
