package lila.openingexplorer

import chess.Color

case class GameRef(
    gameId: String,
    rating: Int,
    winner: Option[Color]) extends PackHelper {

  def pack: Array[Byte] = {
    val base = ('0' to '9') ++ ('A' to 'Z') ++ ('a' to 'z')
    val powers = (0 to 7).map(math.pow(base.size, _).toLong)
    val packedGameId = (gameId, powers).zipped.map(base.indexOf(_) * _).sum

    val packedWinner = winner match {
      case Some(Color.White) => 2 << 14
      case Some(Color.Black) => 1 << 14
      case None              => 0
    }

    packUint48(packedGameId) ++ packUint16(packedWinner | rating)
  }

}

object GameRef extends PackHelper {

  def unpack(packed: Array[Byte]): GameRef = {
    val winnerXorRating = unpackUint16(packed.drop(6))

    val rating = winnerXorRating & 0x3fff

    val winner = winnerXorRating >> 14 match {
      case 2 => Some(Color.White)
      case 1 => Some(Color.Black)
      case _ => None
    }

    GameRef("00000000", rating, winner)
  }

}
