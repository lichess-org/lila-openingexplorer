package lila.openingexplorer

import fm.last.commons.kyoto.factory.{ KyotoDbBuilder, Mode, Compressor, PageComparator, LogLevel, LogAppender }
import java.io.File

object Kyoto {

  def builder(dbFile: File) = new KyotoDbBuilder(dbFile)
    .logLevel(LogLevel.DEBUG)
    .logAppender(LogAppender.STDOUT)
}
