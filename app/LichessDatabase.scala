package lila.openingexplorer

import java.io.File

import fm.last.commons.kyoto.factory.{ Mode, PageComparator }
import fm.last.commons.kyoto.{ KyotoDb, WritableVisitor }

import chess.variant.Variant
import chess.{ Hash, PositionHash, Situation, MoveOrDrop }

final class LichessDatabase extends LichessDatabasePacker {

  val variants = Variant.all.filter(chess.variant.FromPosition!=)

  private val dbs: Map[Variant, KyotoDb] = variants.map({
    case variant => variant -> Util.wrapLog(
      s"Loading ${variant.name} database...",
      s"${variant.name} database loaded!") {
        val config = Config.explorer.lichess(variant)
        val dbFile = new File(config.kyoto.file.replace("(variant)", variant.key))
        dbFile.createNewFile
        Kyoto.builder(dbFile)
          .modes(Mode.CREATE, Mode.READ_WRITE)
          .buckets(config.kyoto.buckets)
          .memoryMapSize(config.kyoto.memoryMapSize)
          .defragUnitSize(config.kyoto.defragUnitSize)
          .buildAndOpen
      }
  }).toMap

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
    val entry = probe(situation)
    val groups = Entry.groups(request.ratings, request.speeds)
    val gameRefs = entry.gameRefs(groups)

    val potentialTopGames =
      entry.gameRefs(Entry.groups(RatingGroup.all, request.speeds))
        .sortWith(_.averageRating > _.averageRating)
        .take(math.min(request.topGames, LichessDatabasePacker.maxTopGames))

    val highestRatingGroup =
      potentialTopGames.headOption.map { bestGame =>
        RatingGroup.find(bestGame.averageRating)
      }

    // only yield top games if highest rating group selected
    val topGames =
      if (highestRatingGroup.fold(false) { request.ratings.contains _ })
        potentialTopGames.filter { game =>
          request.ratings.contains(RatingGroup.find(game.averageRating))
        }
      else
        List.empty

    val numRecentGames =
      math.max(
        LichessDatabasePacker.maxRecentGames,
        gameRefs.size - request.speeds.size * LichessDatabasePacker.maxTopGames
      )

    new QueryResult(
      entry.whiteWins(groups),
      entry.draws(groups),
      entry.blackWins(groups),
      entry.averageRating(groups),
      gameRefs.take(math.min(request.recentGames, numRecentGames)),
      topGames)
  }

  def queryChildren(situation: Situation, request: Request): Children = {
    val subRequest = request.withoutGames
    Util.situationMovesOrDrops(situation).map { move =>
      move -> query(move.fold(_.situationAfter, _.situationAfter), subRequest)
    }.toList
  }

  def merge(variant: Variant, gameRef: GameRef, hashes: Array[PositionHash]) = dbs get variant foreach { db =>
    val freshRecord = pack(Entry.fromGameRef(gameRef))

    db.accept(hashes, new WritableVisitor {
      def record(key: PositionHash, value: Array[Byte]): Array[Byte] = {
        pack(unpack(value).withGameRef(gameRef))
      }

      def emptyRecord(key: PositionHash): Array[Byte] = freshRecord
    })
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

  def closeAll = {
    dbs.values.foreach { db =>
      db.close()
    }
  }
}

object LichessDatabase {

  case class Request(
      speeds: List[SpeedGroup],
      ratings: List[RatingGroup],
      topGames: Int,
      recentGames: Int) {

    def withoutGames = copy(topGames = 0, recentGames = 0)
  }

  val hash = new Hash(32) // 128 bit Zobrist hasher

}
