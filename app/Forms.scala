package lila.openingexplorer

import play.api.data._
import play.api.data.Forms._

object Forms {

  private val moves = (1 to 20)
  private val movesDefault = 12

  private val variants = List("standard", "crazyhouse")
  private val speeds = List("bullet", "blitz", "classical")
  private val ratings = RatingGroup.all.map(_.range.min)

  object master {

    case class Data(fen: String, moves: Option[Int]) {

      def movesOrDefault = moves getOrElse movesDefault
    }

    val form = Form(mapping(
      "fen" -> nonEmptyText,
      "moves" -> optional(number.verifying(moves contains _))
    )(Data.apply)(Data.unapply))
  }

  object lichess {

    case class Data(
        fen: String,
        moves: Option[Int],
        variant: String,
        speeds: List[String],
        ratings: List[Int]) {

      def ratingGroups = RatingGroup.all.filter { rg =>
        ratings contains rg.range.min
      }

      def movesOrDefault = moves getOrElse movesDefault
    }

    val form = Form(mapping(
      "fen" -> nonEmptyText,
      "moves" -> optional(number.verifying(moves contains _)),
      "variant" -> nonEmptyText.verifying(variants contains _),
      "speeds" -> list(nonEmptyText.verifying(speeds contains _)),
      "ratings" -> list(number.verifying(ratings contains _))
    )(Data.apply)(Data.unapply))
  }
}
