package controllers

import play.api._
import play.api.mvc_

class Application extends Controller {

  def index = Action {
    Ok(views.html.index("Hello world!"))
  }

}
