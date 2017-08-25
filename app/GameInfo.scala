package lila.openingexplorer

import chess.format.pgn.{ Parser, Tag }

case class GameInfo(
    white: GameInfo.Player,
    black: GameInfo.Player,
    year: Option[Int]
)

object GameInfo {

  case class Player(name: String, rating: Int)

  private val YearRegex = s".*(\\d{4}).*".r

  def parse(pgn: String): Option[GameInfo] = try {
    Parser.TagParser.fromFullPgn(pgn).toOption flatMap parse
  } catch {
    case e: StackOverflowError =>
      println(pgn)
      println(s"### StackOverflowError ### in GameInfo.parse")
      None
  }

  def parse(tags: List[Tag]): Option[GameInfo] = {
    def find(name: String): Option[String] = tags.find(_.name.name == name).map(_.value)
    for {
      whiteName <- find("White")
      whiteRating <- find("WhiteElo") flatMap parseIntOption
      blackName <- find("Black")
      blackRating <- find("BlackElo") flatMap parseIntOption
      year = find("Date") flatMap {
        case YearRegex(y) => parseIntOption(y)
        case _ => None
      }
    } yield GameInfo(
      white = Player(whiteName, whiteRating),
      black = Player(blackName, blackRating),
      year = year
    )
  }
}
