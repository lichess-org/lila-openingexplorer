package lila.openingexplorer

import io.methvin.play.autoconfig._
import play.api._
import javax.inject.{ Inject, Singleton }
import scala.concurrent.duration._

import chess.variant._

@Singleton
final class Config @Inject() (conf: Configuration) {

  import Config._

  lazy val explorer = conf.get[Explorer]("explorer")
}
object Config {

  case class Explorer(
      master: Domain,
      lichess: Lichess,
      pgn: Pgn,
      gameInfo: GameInfo,
      corsHeader: Boolean,
      cache: Cache
  )
  implicit val explorerLoader: ConfigLoader[Explorer] = AutoConfig.loader

  case class Cache(
      maxMoves: Int,
      ttl: FiniteDuration
  )
  implicit val cacheLoader: ConfigLoader[Cache] = AutoConfig.loader

  case class Domain(
      maxPlies: Int,
      kyoto: Kyoto
  )
  implicit val domainLoader: ConfigLoader[Domain] = AutoConfig.loader

  case class Pgn(kyoto: Kyoto)
  implicit val pgnLoader: ConfigLoader[Pgn] = AutoConfig.loader

  case class GameInfo(kyoto: Kyoto)
  implicit val gameInfoLoader: ConfigLoader[GameInfo] = AutoConfig.loader

  case class Kyoto(
      file: String,
      buckets: Long,
      defragUnitSize: Int,
      memoryMapSize: Long
  )
  implicit val kyotoLoader: ConfigLoader[Kyoto] = AutoConfig.loader

  case class Lichess(
      standard: Domain,
      chess960: Domain,
      kingOfTheHill: Domain,
      threeCheck: Domain,
      antichess: Domain,
      atomic: Domain,
      horde: Domain,
      racingKings: Domain,
      crazyhouse: Domain
  ) {

    def apply(variant: Variant): Domain =
      variant match {
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
  implicit val lichessLoader: ConfigLoader[Lichess] = AutoConfig.loader
}
