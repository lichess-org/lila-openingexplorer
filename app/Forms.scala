package lila.openingexplorer

import play.api.data._
import play.api.data.Forms._

object Forms {

  private val movesDefault = 12

  private val variants = chess.variant.Variant.all.map(_.key)
  private val speeds = SpeedGroup.all.map(_.name)
  private val ratings = RatingGroup.all.map(_.range.min)

  object master {

    case class Data(fen: String, moves: Option[Int]) {

      def movesOrDefault = moves getOrElse movesDefault
    }

    val form = Form(mapping(
      "fen" -> nonEmptyText,
      "moves" -> optional(number)
    )(Data.apply)(Data.unapply))
  }

  object lichess {

    case class Data(
        fen: String,
        moves: Option[Int],
        variant: String,
        speeds: List[String],
        ratings: List[Int]) {

      def ratingGroups = RatingGroup.all.filter { x =>
        ratings contains x.range.min
      }

      def speedGroups = SpeedGroup.all.filter { x =>
        speeds contains x.name
      }

      def actualVariant = chess.variant.Variant orDefault variant

      def movesOrDefault = moves getOrElse movesDefault

      def fullHouse = speeds == Forms.speeds && ratings == Forms.ratings
    }

    val form = Form(mapping(
      "fen" -> nonEmptyText,
      "moves" -> optional(number),
      "variant" -> nonEmptyText.verifying(variants contains _),
      "speeds" -> list(nonEmptyText.verifying(speeds contains _)),
      "ratings" -> list(number.verifying(ratings contains _))
    )(Data.apply)(Data.unapply))
  }
}
