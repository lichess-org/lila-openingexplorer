name := """lila-openingexplorer"""

version := "1.0"

lazy val chess = project in file("chess")

lazy val root = project in file(".") enablePlugins PlayScala dependsOn chess settings (
  sources in doc in Compile := List(),
  publishArtifact in (Compile, packageDoc) := false,
  publishArtifact in (Compile, packageSrc) := false)

scalaVersion := "2.11.7"

scalacOptions ++= Seq("-unchecked", "-language:_")

libraryDependencies ++= Seq(
  "com.github.ornicar" %% "scalalib" % "5.3",
  "fm.last.commons" % "lastcommons-kyoto" % "1.24.0",
  "com.github.kxbmap" %% "configs" % "0.3.0",
  "com.baqend" % "bloom-filter" % "1.01",
  cache,
  ws
)

resolvers += "scalaz-bintray" at "http://dl.bintray.com/scalaz/releases"

// Play provides two styles of routers, one expects its actions to be injected,
// the other, legacy style, accesses its actions statically.
routesGenerator := InjectedRoutesGenerator
