package lila.openingexplorer

import fm.last.commons.kyoto.{ KyotoDb, WritableVisitor }
import java.io.File
import java.io.{ ByteArrayInputStream, ByteArrayOutputStream }
import javax.inject.{ Inject, Singleton }
import akka.actor.CoordinatedShutdown

import chess.variant.Variant
import chess.{ Hash, MoveOrDrop, PositionHash, Situation }

@Singleton
final class LichessDatabase @Inject() (
    config: Config,
    shutdown: CoordinatedShutdown
)(implicit ec: scala.concurrent.ExecutionContext) {

  val variants = Variant.all.filter(chess.variant.FromPosition.!=)

  private val dbs: Map[Variant, KyotoDb] = variants
    .map({
      case variant =>
        variant -> Util.wrapLog(
          s"Loading ${variant.name} database...",
          s"${variant.name} database loaded!"
        ) {
          val conf   = config.explorer.lichess(variant)
          val dbFile = new File(conf.kyoto.file.replace("(variant)", variant.key))
          dbFile.createNewFile
          Kyoto.builder(dbFile, conf.kyoto).buildAndOpen
        }
    })
    .toMap

  import LichessDatabase.Request

  private def probe(situation: Situation): Entry =
    probe(situation.board.variant, LichessDatabase.hash(situation))

  private def probe(variant: Variant, h: PositionHash): Entry = {
    dbs.get(variant).flatMap(db => Option(db.get(h))) match {
      case Some(bytes) => unpack(bytes)
      case None        => Entry.empty
    }
  }

  def query(situation: Situation, request: Request): QueryResult = {
    val entry    = probe(situation)
    val groups   = Entry.groups(request.ratings, request.speeds)
    val gameRefs = entry.gameRefs(groups)

    val potentialTopGames =
      entry
        .gameRefs(Entry.groups(RatingGroup.all, request.speeds))
        .sortWith(_.averageRating > _.averageRating)
        .take(math.min(request.topGames, Entry.maxTopGames))

    val highestRatingGroup =
      potentialTopGames.headOption.map { bestGame =>
        RatingGroup.find(bestGame.averageRating)
      }

    // only yield top games if highest rating group selected
    val topGames =
      if (highestRatingGroup.fold(false) { request.ratings.contains _ })
        potentialTopGames.filter { game =>
          request.ratings.contains(RatingGroup.find(game.averageRating))
        } else
        List.empty

    val numRecentGames =
      math.max(
        Entry.maxRecentGames,
        gameRefs.size - request.speeds.size * Entry.maxTopGames
      )

    new QueryResult(
      entry.whiteWins(groups),
      entry.draws(groups),
      entry.blackWins(groups),
      entry.averageRating(groups),
      entry
        .moves(groups)
        .toList
        .filterNot(_._2.isEmpty)
        .sortBy(-_._2.total)
        .take(request.maxMoves)
        .flatMap {
          case (uci, stats) => Util.moveFromUci(situation, uci).map(_ -> stats)
        },
      gameRefs.take(math.min(request.recentGames, numRecentGames)),
      topGames
    )
  }

  private def unpack(b: Array[Byte]): Entry = {
    val in = new ByteArrayInputStream(b)
    Entry.read(in)
  }

  def merge(variant: Variant, gameRef: GameRef, move: MoveOrDrop) = dbs get variant foreach { db =>
    val hash = LichessDatabase.hash(move.fold(_.situationBefore, _.situationBefore))
    val uci  = move.left.map(_.toUci).map(_.toUci)

    db.accept(
      hash,
      new WritableVisitor {
        def record(key: PositionHash, value: Array[Byte]): Array[Byte] = {
          val out = new ByteArrayOutputStream()
          unpack(value).withGameRef(gameRef, uci).write(out)
          out.toByteArray
        }

        def emptyRecord(key: PositionHash): Array[Byte] = {
          val out = new ByteArrayOutputStream()
          Entry.fromGameRef(gameRef, uci).write(out)
          out.toByteArray
        }
      }
    )
  }

  def uniquePositions(variant: Variant): Long =
    dbs.get(variant).map(_.recordCount()).getOrElse(0L)

  def totalGames(variant: Variant): Long = {
    val games = (chess.format.Forsyth << variant.initialFen)
      .map((situation) => probe(situation withVariant variant).totalGames)
      .getOrElse(0L)

    if (variant == chess.variant.Chess960)
      // by rule of thumb ...
      games * 960
    else
      games
  }

  shutdown.addTask(CoordinatedShutdown.PhaseServiceStop, "close master db") { () =>
    scala.concurrent.Future {
      dbs.values.foreach { db =>
        db.close()
      }
      akka.Done
    }
  }
}

object LichessDatabase {

  case class Request(
      speeds: List[SpeedGroup],
      ratings: List[RatingGroup],
      topGames: Int,
      recentGames: Int,
      maxMoves: Int
  ) {}

  val hash = new Hash(16) // 128 bit Zobrist hasher
}
