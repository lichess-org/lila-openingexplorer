package lila.openingexplorer

case class Category private (name: String, variant: chess.variant.Variant) {

}

object Category {
  val Bullet = new Category("bullet", chess.variant.Standard);
  val Blitz = new Category("blitz", chess.variant.Standard);
  val Standard = new Category("standard", chess.variant.Standard);

  val Crazyhouse = new Category("crazyhouse", chess.variant.Crazyhouse);
  val KotH = new Category("koth", chess.variant.KingOfTheHill);
  val ThreeCheck = new Category("threecheck", chess.variant.ThreeCheck);
  val Antichess = new Category("antichess", chess.variant.Antichess);
  val Atomic = new Category("atomic", chess.variant.Atomic);
  val Horde = new Category("horde", chess.variant.Horde);
  val RacingKings = new Category("racing", chess.variant.RacingKings);

  val all = List(
    Bullet, Blitz, Standard,
    Crazyhouse, KotH, ThreeCheck, Antichess, Atomic, Horde, RacingKings
  );

  def find(name: String): Option[Category] = {
    all.find(_.name == name)
  }
}
