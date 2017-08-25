import play.api._
import play.api.routing.Router
import play.api.cache.SyncCacheApi

import lila.openingexplorer._

class AppLoader extends ApplicationLoader {
  private var components: AppComponents = _

  def load(context: ApplicationLoader.Context): Application = {
    components = new AppComponents(context)
    components.application
  }
}

class AppComponents(context: ApplicationLoader.Context)
  extends BuiltInComponentsFromContext(context) {

  def httpFilters = Nil

  val masterDb = new MasterDatabase()
  val lichessDb = new LichessDatabase()
  val pgnDb = new PgnDatabase()
  val gameInfoDb = new GameInfoDatabase()

  val importer = new Importer(masterDb, lichessDb, pgnDb, gameInfoDb)
  val appCache = new Cache(injector.instanceOf[SyncCacheApi])

  context.lifecycle.addStopHook { () =>
    scala.concurrent.Future.successful {
      masterDb.close
      lichessDb.closeAll
      pgnDb.close
      gameInfoDb.close
    }
  }

  lazy val homeController = new _root_.controllers.WebApi(
    controllerComponents,
    masterDb,
    lichessDb,
    pgnDb,
    gameInfoDb,
    importer,
    appCache
  )

  lazy val router: Router = new _root_.router.Routes(httpErrorHandler, homeController)
}
