package lila

package object openingexplorer {

  /* def parseIntOption(str: String): Option[Int] = {
    try {
      Some(java.lang.Integer.parseInt(str))
    } catch {
      case e: NumberFormatException => None
    }
  } */

  def parseLongOption(str: String): Option[Long] = {
    try {
      Some(java.lang.Long.parseLong(str))
    } catch {
      case e: NumberFormatException => None
    }
  }

}
