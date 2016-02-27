package lila.openingexplorer

import fm.last.commons.kyoto.factory.{ KyotoDbBuilder, Mode, Compressor, PageComparator, LogLevel, LogAppender }
import java.io.File

object Kyoto {

  def builder(dbFile: File) = new KyotoDbBuilder(dbFile)
    .modes(Mode.CREATE, Mode.READ_WRITE, Mode.AUTO_TRANSACTION)
    .logLevel(LogLevel.WARN)
    .logAppender(LogAppender.STDOUT)
}
