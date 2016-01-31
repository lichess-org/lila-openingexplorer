package lila.openingexplorer

import chess.Color

case class GameRef(
    gameId: String,
    rating: Int,
    winner: Option[Color]) extends PackHelper {

  def pack: Array[Byte] = {
    val packedGameId = gameId.zip(gameId.indices.reverse).map {
      case (c, i) =>
        GameRef.base.indexOf(c) * math.pow(GameRef.base.size, i).toLong
    }.sum

    val packedWinner = winner match {
      case Some(Color.White) => 2 << 14
      case Some(Color.Black) => 1 << 14
      case None              => 0
    }

    packUint48(packedGameId) ++ packUint16(packedWinner | rating)
  }

}

object GameRef extends PackHelper {

  val packSize = 8

  private val base = ('0' to '9') ++ ('a' to 'z') ++ ('A' to 'Z')

  def unpack(packed: Array[Byte]): GameRef = {
    val winnerXorRating = unpackUint16(packed.drop(6))

    val rating = winnerXorRating & 0x3fff

    val winner = winnerXorRating >> 14 match {
      case 2 => Some(Color.White)
      case 1 => Some(Color.Black)
      case _ => None
    }

    def decodeGameId(v: Long, res: List[Char] = Nil): List[Char] = {
      val quotient = v / base.size
      if (quotient > 0)
        decodeGameId(quotient, base((v % base.size).toInt) :: res)
      else
        base(v.toInt) :: res
    }

    GameRef(
      decodeGameId(unpackUint48(packed))
        .mkString
        .reverse.padTo(8, base(0)).reverse,
      rating,
      winner
    )
  }

}
