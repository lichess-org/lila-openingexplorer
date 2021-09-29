name := """lila-openingexplorer"""

version := "2.4"

lazy val root = (project in file("."))
  .enablePlugins(PlayScala, JavaAppPackaging)
  .disablePlugins(PlayFilters)

sources in doc in Compile                := List()
publishArtifact in (Compile, packageDoc) := false
publishArtifact in (Compile, packageSrc) := false

scalaVersion := "2.13.6"

scalacOptions ++= Seq(
  "-language:implicitConversions",
  "-feature",
  "-deprecation",
  "-unchecked",
  "-Wdead-code",
  "-Xlint:unused,inaccessible,nullary-unit,adapted-args,infer-any,missing-interpolator,eta-zero",
  "-Xfatal-warnings"
)

libraryDependencies ++= Seq(
  "org.lichess"            %% "scalachess"                 % "10.2.9",
  "fm.last.commons"         % "lastcommons-kyoto"          % "1.24.0",
  "com.github.blemale"     %% "scaffeine"                  % "4.0.2" % "compile",
  "io.methvin.play"        %% "autoconfig-macros"          % "0.3.2" % "provided",
  "org.scala-lang.modules" %% "scala-parallel-collections" % "1.0.4",
  guice,
  specs2 % Test
)

resolvers += "lila-maven" at "https://raw.githubusercontent.com/ornicar/lila-maven/master"

import play.sbt.routes.RoutesKeys
RoutesKeys.routesImport := Seq.empty

// Play provides two styles of routers, one expects its actions to be injected,
// the other, legacy style, accesses its actions statically.
routesGenerator := InjectedRoutesGenerator
