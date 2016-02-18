package lila.openingexplorer

import play.api.data._
import play.api.data.Forms._

object Forms {

  private val movesDefault = 12
  private val topGamesDefault = 4
  private val topGamesMax = 4
  private val recentGamesDefault = 4
  private val recentGamesMax = 10

  private val variants = chess.variant.Variant.all.map(_.key)
  private val speeds = SpeedGroup.all.map(_.name)
  private val ratings = RatingGroup.all.map(_.range.min)

  object master {

    case class Data(fen: String, moves: Option[Int], topGames: Option[Int]) {

      def movesOrDefault = moves getOrElse movesDefault
      def topGamesOrDefault = topGames getOrElse topGamesDefault
    }

    val form = Form(mapping(
      "fen" -> nonEmptyText,
      "moves" -> optional(number),
      "topGames" -> optional(number)
    )(Data.apply)(Data.unapply))
  }

  object lichess {

    case class Data(
        fen: String,
        moves: Option[Int],
        variant: String,
        speeds: List[String],
        ratings: List[Int],
        topGames: Option[Int],
        recentGames: Option[Int]) {

      def ratingGroups = RatingGroup.all.filter { x =>
        ratings contains x.range.min
      }

      def speedGroups = SpeedGroup.all.filter { x =>
        speeds contains x.name
      }

      def actualVariant = chess.variant.Variant orDefault variant

      def movesOrDefault = moves getOrElse movesDefault
      def topGamesOrDefault = topGames getOrElse topGamesDefault
      def recentGamesOrDefault = recentGames getOrElse recentGamesDefault

      def fullHouse = speeds == Forms.speeds && ratings == Forms.ratings
    }

    val form = Form(mapping(
      "fen" -> nonEmptyText,
      "moves" -> optional(number),
      "variant" -> nonEmptyText.verifying(variants contains _),
      "speeds" -> list(nonEmptyText.verifying(speeds contains _)),
      "ratings" -> list(number.verifying(ratings contains _)),
      "topGames" -> optional(number(min = 0, max = topGamesMax)),
      "recentGames" -> optional(number(min = 0, max = recentGamesMax))
    )(Data.apply)(Data.unapply))
  }
}
