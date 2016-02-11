package lila.openingexplorer

import com.github.kxbmap.configs.syntax._
import com.typesafe.config.ConfigFactory

import chess.variant._

object Config {

  val explorer = ConfigFactory.load.get[Explorer]("explorer")

  case class Explorer(
    master: Domain,
    lichess: Lichess,
    pgn: Pgn,
    gameInfo: GameInfo,
    bloomFilter: BloomFilter,
    corsHeader: Boolean)

  case class Domain(
    maxPlies: Int,
    kyoto: Kyoto)

  case class Pgn(kyoto: KyotoMin)

  case class GameInfo(kyoto: KyotoMin)

  case class KyotoMin(
    buckets: Long,
    defragUnitSize: Int)

  case class Kyoto(
    buckets: Long,
    defragUnitSize: Int,
    memory: Memory)

  case class Memory(
    mapSize: Long,
    pageCacheSize: Long)

  case class Lichess(
      standard: Domain,
      chess960: Domain,
      kingOfTheHill: Domain,
      threeCheck: Domain,
      antichess: Domain,
      atomic: Domain,
      horde: Domain,
      racingKings: Domain,
      crazyhouse: Domain) {

    def apply(variant: Variant): Domain = variant match {
      case Standard      => standard
      case Chess960      => chess960
      case KingOfTheHill => kingOfTheHill
      case ThreeCheck    => threeCheck
      case Antichess     => antichess
      case Atomic        => atomic
      case Horde         => horde
      case RacingKings   => racingKings
      case Crazyhouse    => crazyhouse
    }
  }

  case class BloomFilter(
    expectedGames: Int,
    acceptableError: Double,
    persistent: Boolean)
}
