package lila.openingexplorer

import org.specs2.mutable._

import chess.format.{ FEN, Forsyth }

class HashTest extends Specification {

  "hashes" should {

    "be consistent" in {
      val sit = Forsyth << FEN("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
      sit.map(LichessDatabase.hash.apply(_).map("%02x" format _).mkString) mustEqual Some(
        "463b96181691fc9c3d71fe83987aab73"
      )
    }
  }
}
