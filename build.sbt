name := """lila-openingexplorer"""

version := "2.2"

lazy val root = (project in file("."))
  .enablePlugins(PlayScala, JavaAppPackaging)
  // .enablePlugins(PlayScala, PlayNettyServer, JavaAppPackaging)
  // .disablePlugins(PlayAkkaHttpServer)
  .disablePlugins(PlayFilters)

sources in doc in Compile := List()
publishArtifact in (Compile, packageDoc) := false
publishArtifact in (Compile, packageSrc) := false

scalaVersion := "2.12.4"

scalacOptions ++= Seq("-unchecked", "-language:_")

// https://groups.google.com/d/msg/specs2-users/7rvENck2Nzw/N6F-Q5EGv0oJ
testOptions in Test += Tests.Setup(() => System.setProperty("java.vm.vendor", "Sun"))

libraryDependencies ++= Seq(
  "org.lichess" %% "scalachess" % "8.2",
  "com.github.ornicar" %% "scalalib" % "6.5",
  "fm.last.commons" % "lastcommons-kyoto" % "1.24.0",
  "com.github.kxbmap" %% "configs" % "0.4.4",
  "com.github.blemale" %% "scaffeine" % "2.2.0" % "compile",
  specs2 % Test
)

resolvers += "scalaz-bintray" at "http://dl.bintray.com/scalaz/releases"
resolvers += "lila-maven" at "https://raw.githubusercontent.com/ornicar/lila-maven/master"

testOptions in Test := Seq(Tests.Argument(TestFrameworks.Specs2, "console"))

// Play provides two styles of routers, one expects its actions to be injected,
// the other, legacy style, accesses its actions statically.
routesGenerator := InjectedRoutesGenerator

import com.typesafe.sbt.SbtScalariform.autoImport.scalariformFormat
import com.typesafe.sbt.SbtScalariform.ScalariformKeys
import scalariform.formatter.preferences._

Seq(
  ScalariformKeys.preferences := ScalariformKeys.preferences.value
    .setPreference(DanglingCloseParenthesis, Force)
    .setPreference(DoubleIndentConstructorArguments, true),
  excludeFilter in scalariformFormat := "*Routes*"
)
