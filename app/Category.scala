package lila.openingexplorer

case class Category private (name: String) {

}

object Category {
  val Bullet = new Category("bullet");
  val Blitz = new Category("blitz");
  val Standard = new Category("standard");

  val CrazyHouse = new Category("crazyhouse");
  val KotH = new Category("koth");
  val ThreeCheck = new Category("threecheck");
  val Antichess = new Category("antichess");
  val Atomic = new Category("atomic");
  val Horde = new Category("horde");
  val RacingKings = new Category("racing");

  val all = List(
    Bullet, Blitz, Standard,
    CrazyHouse, KotH, ThreeCheck, Antichess, Atomic, Horde, RacingKings
  );

  def find(name: String): Option[Category] = {
    all.find(_.name == name)
  }
}
