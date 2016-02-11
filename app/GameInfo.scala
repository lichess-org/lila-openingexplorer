package lila.openingexplorer

import chess.format.pgn.Parser

case class GameInfo(
  white: GameInfo.Player,
  black: GameInfo.Player,
  year: Option[Int],
  result: Option[chess.Color])

object GameInfo {

  case class Player(name: String, rating: Int)

  private val YearRegex = s".*(\\d{4}).*".r

  def parse(pgn: String): Option[GameInfo] =
    Parser.TagParser(pgn).toOption flatMap { tags =>
      def find(name: String): Option[String] = tags.find(_.name.name == name).map(_.value)
      for {
        whiteName <- find("White")
        whiteRating <- find("WhiteElo") flatMap parseIntOption
        blackName <- find("Black")
        blackRating <- find("BlackElo") flatMap parseIntOption
        result = find("Result") flatMap {
          case "1-0" => chess.White.some
          case "0-1" => chess.Black.some
          case _     => None
        }
        year = find("Date") flatMap {
          case YearRegex(y) => parseIntOption(y)
          case _            => None
        }
      } yield GameInfo(
        white = Player(whiteName, whiteRating),
        black = Player(blackName, blackRating),
        result = result,
        year = year)
    }
}
