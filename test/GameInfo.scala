package lila.openingexplorer

import org.specs2.mutable._

import chess.Color

class GameInfoTest extends Specification {

  "parse" should {

    "normalized PGN" in {
      val pgn = """
[Event "Moscow Tal Memorial Blitz"]
[Site "Moscow"]
[Date "2008.08.29"]
[Round "17"]
[White "Morozevich, Alexander"]
[Black "Karjakin, Sergey"]
[Result "0-1"]
[WhiteElo "2788"]
[BlackElo "2727"]

1. e4 e5 2. Nf3 Nc6 3. Bc4 Nf6 4. Ng5 d5 5. exd5 Na5 6. Bb5+ c6 7. dxc6 bxc6 8. Bd3 Be7 9. Nc3 O-O 10. O-O h6 11. Nf3 Bg4 12. h3 Bh5 13. Be2 e4 14. Ne5 Bxe2 15. Qxe2 Qd4 16. Ng4 Rfe8 17. d3 exd3 18. Qxd3 Qxd3 19. cxd3 Nxg4 20. hxg4 Rad8 21. Rd1 Bc5 22. Bf4 Rd7 23. Rac1 Bb6 24. Na4 Rd4 25. Nxb6 Rxf4 1/2-1/2
"""
      GameInfo.parse(pgn) must_== Some(GameInfo(
        white = GameInfo.Player("Morozevich, Alexander", 2788),
        black = GameInfo.Player("Karjakin, Sergey", 2727),
        result = Some(chess.Black),
        year = Some(2008)))
    }
  }
}
