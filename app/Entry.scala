package lila.openingexplorer

import java.io.{ InputStream, OutputStream }

import scalaz._
import Scalaz._

import chess.format.Uci

case class Entry(sub: Map[(RatingGroup, SpeedGroup), SubEntry]) extends PackHelper {

  def subEntry(ratingGroup: RatingGroup, speedGroup: SpeedGroup): SubEntry =
    sub.getOrElse((ratingGroup, speedGroup), SubEntry.empty)

  def subEntries(groups: List[(RatingGroup, SpeedGroup)]): List[SubEntry] =
    groups.map((g) => subEntry(g._1, g._2))

  def totalGames = sub.values.map(_.totalGames).sum

  def withGameRef(game: GameRef, move: Either[Uci.Move, Uci.Drop]): Entry = {
    val ratingGroup = RatingGroup.find(game.averageRating)
    copy(sub = sub + ((ratingGroup, game.speed) -> subEntry(ratingGroup, game.speed).withGameRef(game, move)))
  }

  def withExistingGameRef(game: GameRef): Entry = {
    val ratingGroup = RatingGroup.find(game.averageRating)
    copy(sub = sub + ((ratingGroup, game.speed) -> subEntry(ratingGroup, game.speed).withExistingGameRef(game)))
  }

  def gameRefs(groups: List[(RatingGroup, SpeedGroup)]): List[GameRef] =
    subEntries(groups)
      .map(_.gameRefs)
      .flatMap(_.zipWithIndex).sortBy(_._2).map(_._1)  // interleave

  def whiteWins(groups: List[(RatingGroup, SpeedGroup)]): Long =
    subEntries(groups).map(_.totalWhite).sum

  def draws(groups: List[(RatingGroup, SpeedGroup)]): Long =
    subEntries(groups).map(_.totalDraws).sum

  def blackWins(groups: List[(RatingGroup, SpeedGroup)]): Long =
    subEntries(groups).map(_.totalBlack).sum

  def averageRatingSum(groups: List[(RatingGroup, SpeedGroup)]): Long =
    subEntries(groups).map(_.totalAverageRatingSum).sum

  def numGames(groups: List[(RatingGroup, SpeedGroup)]): Long =
    subEntries(groups).map(_.totalGames).sum

  def averageRating(groups: List[(RatingGroup, SpeedGroup)]): Int = {
    val games = numGames(groups)
    if (games == 0) 0 else (averageRatingSum(groups) / games).toInt
  }

  def moves(groups: List[(RatingGroup, SpeedGroup)]): Map[Either[Uci.Move, Uci.Drop], MoveStats] = {
    implicit val merge: Semigroup[MoveStats] = Semigroup.instance((a, b) => a.add(b))
    subEntries(groups).map(_.moves).foldLeft(Map.empty[Either[Uci.Move, Uci.Drop], MoveStats])(_ |+| _)
  }

  lazy val allGameRefs = gameRefs(Entry.allGroups)
  def totalWhiteWins = whiteWins(Entry.allGroups)
  def totalDraws = draws(Entry.allGroups)
  def totalBlackWins = blackWins(Entry.allGroups)
  def totalAverageRatingSum = averageRatingSum(Entry.allGroups)

  def write(out: OutputStream) = {
    val topGameRefs = SpeedGroup.all.flatMap { speed =>
      gameRefs(Entry.groups(speed))
        .sortWith(_.averageRating > _.averageRating)
        .take(Entry.maxTopGames)
    }

    sub.foreach { case (group, subEntry) =>
      val toBeStored =
        (subEntry.gameRefs.take(Entry.maxRecentGames) ::: topGameRefs.filter(_.group == group))
          .distinct

      if (toBeStored.size > 0) {
        writeUint(out, toBeStored.size)
        toBeStored.foreach(_.write(out))
        subEntry.writeStats(out)
      }
    }
  }
}

object Entry extends PackHelper {

  val maxTopGames = 4

  val maxRecentGames = 2

  def read(in: InputStream): Entry = {
    val subEntries = scala.collection.mutable.Map.empty[(RatingGroup, SpeedGroup), SubEntry]

    while (in.available > 0) {
      var remainingRefs = readUint(in)
      if (remainingRefs > 0) {
        // the first game ref is used to select the group
        val gameRef = GameRef.read(in)
        remainingRefs -= 1;

        val gameRefs = scala.collection.mutable.ListBuffer.empty[GameRef]
        gameRefs += gameRef

        while (remainingRefs > 0) {
          gameRefs += GameRef.read(in)
          remainingRefs -= 1
        }

        subEntries += (gameRef.group -> SubEntry.readStats(in, gameRefs.toList))
      }
    }

    Entry(subEntries.toMap)
  }

  def empty = Entry(Map.empty)

  def fromGameRef(game: GameRef, move: Either[Uci.Move, Uci.Drop]) =
    Entry.empty.withGameRef(game, move)

  def groups(
    ratings: List[RatingGroup],
    speeds: List[SpeedGroup]): List[(RatingGroup, SpeedGroup)] = {
    // cross product
    for {
      ratingGroup <- ratings
      speedGroup <- speeds
    } yield (ratingGroup, speedGroup)
  }

  def groups(speed: SpeedGroup): List[(RatingGroup, SpeedGroup)] =
    groups(RatingGroup.all, List(speed))

  val allGroups = groups(RatingGroup.all, SpeedGroup.all)
}
