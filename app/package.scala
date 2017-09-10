package lila

package object openingexplorer {

  def parseIntOption(str: String): Option[Int] = {
    try {
      Some(java.lang.Integer.parseInt(str))
    } catch {
      case e: NumberFormatException => None
    }
  }

  def parseLongOption(str: String): Option[Long] = {
    try {
      Some(java.lang.Long.parseLong(str))
    } catch {
      case e: NumberFormatException => None
    }
  }

  implicit final class ornicarAddKcombinator[A](any: A) {
    def kCombinator(sideEffect: A ⇒ Unit): A = {
      sideEffect(any)
      any
    }
    def ~(sideEffect: A ⇒ Unit): A = kCombinator(sideEffect)
    def pp: A = kCombinator(println)
    def pp(msg: String): A = kCombinator(a ⇒ println(s"[$msg] $a"))
  }
}
