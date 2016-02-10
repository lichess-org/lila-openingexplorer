package lila.openingexplorer

import com.github.kxbmap.configs.syntax._
import com.typesafe.config.ConfigFactory

object Config {

  val explorer = ConfigFactory.load.get[Explorer]("explorer")

  case class Explorer(
    master: Domain,
    lichess: Lichess)

  case class Domain(
    maxPlies: Int,
    kyoto: Kyoto)

  case class Kyoto(
    buckets: Long,
    memory: Memory)

  case class Memory(
    mapSize: Long,
    pageSize: Long)

  case class Lichess(
    standard: Domain,
    chess960: Domain,
    kingOfTheHill: Domain,
    threeCheck: Domain,
    antichess: Domain,
    atomic: Domain,
    horde: Domain,
    racingKings: Domain,
    crazyhouse: Domain)
}
