package lila.openingexplorer

import fm.last.commons.kyoto.factory.{ KyotoDbBuilder, LogAppender, LogLevel, Mode }
import java.io.File

object Kyoto {

  def builder(config: Config.Kyoto): KyotoDbBuilder = {
    val dbFile = new File(config.file)
    dbFile.createNewFile
    builder(dbFile, config)
  }

  def builder(dbFile: File, config: Config.Kyoto): KyotoDbBuilder =
    new KyotoDbBuilder(dbFile)
      .modes(Mode.CREATE, Mode.READ_WRITE, Mode.AUTO_TRANSACTION)
      .logLevel(LogLevel.WARN)
      .logAppender(LogAppender.STDOUT)
      .buckets(config.buckets)
      .memoryMapSize(config.memoryMapSize)
      .defragUnitSize(config.defragUnitSize)
}
