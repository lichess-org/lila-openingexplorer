name := """lila-openingexplorer"""

version := "2.0"

lazy val chess = project in file("chess")

lazy val root = project in file(".") enablePlugins(PlayScala, JavaAppPackaging) dependsOn chess settings (
  sources in doc in Compile := List(),
  publishArtifact in (Compile, packageDoc) := false,
  publishArtifact in (Compile, packageSrc) := false)

scalaVersion := "2.11.8"

scalacOptions ++= Seq("-unchecked", "-language:_")

libraryDependencies ++= Seq(
  "com.github.ornicar" %% "scalalib" % "5.5",
  "fm.last.commons" % "lastcommons-kyoto" % "1.24.0",
  "com.github.kxbmap" %% "configs" % "0.3.0",
  cache
)

resolvers += "scalaz-bintray" at "http://dl.bintray.com/scalaz/releases"

// Play provides two styles of routers, one expects its actions to be injected,
// the other, legacy style, accesses its actions statically.
routesGenerator := InjectedRoutesGenerator
