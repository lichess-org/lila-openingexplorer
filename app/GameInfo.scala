package lila.openingexplorer

import chess.format.pgn.{ Parser, Tags }

case class GameInfo(
    white: GameInfo.Player,
    black: GameInfo.Player,
    year: Option[Int]
)

object GameInfo {

  case class Player(name: String, rating: Int)

  def parse(pgn: String): Option[GameInfo] =
    try {
      Parser.TagParser.fromFullPgn(pgn).toOption flatMap parse
    } catch {
      case e: StackOverflowError =>
        println(pgn)
        println(s"### StackOverflowError ### in GameInfo.parse")
        None
    }

  def parse(tags: Tags): Option[GameInfo] = {
    for {
      whiteName   <- tags("White")
      whiteRating <- tags("WhiteElo").flatMap(_.toIntOption)
      blackName   <- tags("Black")
      blackRating <- tags("BlackElo").flatMap(_.toIntOption)
      year = tags.year
    } yield GameInfo(
      white = Player(whiteName, whiteRating),
      black = Player(blackName, blackRating),
      year = year
    )
  }
}
