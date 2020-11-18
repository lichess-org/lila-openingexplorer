package lila.openingexplorer

import chess.Clock

sealed abstract class SpeedGroup(
    val id: Int,
    val name: String,
    val range: Range
) {}

object SpeedGroup {

  case object Bullet    extends SpeedGroup(0, "bullet", 0 to 179)
  case object Blitz     extends SpeedGroup(1, "blitz", 180 to 479)
  case object Rapid     extends SpeedGroup(2, "rapid", 480 to 1499)
  case object Classical extends SpeedGroup(3, "classical", 1500 to Int.MaxValue)

  val all = List(Bullet, Blitz, Rapid, Classical)

  val byId = all.view.map { v => (v.id, v) }.toMap

  def apply(speed: chess.Speed) = speed match {
    case chess.Speed.Bullet | chess.Speed.UltraBullet       => Bullet
    case chess.Speed.Blitz                                  => Blitz
    case chess.Speed.Rapid                                  => Rapid
    case chess.Speed.Classical | chess.Speed.Correspondence => Classical
  }

  def fromPgn(tags: chess.format.pgn.Tags): Option[SpeedGroup] = tags("TimeControl") match {
    case Some("-")         => Some(Classical) // correspondence
    case Some(timeControl) => Clock.readPgnConfig(timeControl).map(clock => apply(chess.Speed(clock)))
    case None              => None
  }
}
