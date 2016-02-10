Opening explorer for lichess.org
================================

[![Build Status](https://travis-ci.org/niklasf/lila-openingexplorer.svg?branch=master)](https://travis-ci.org/niklasf/lila-openingexplorer)

Preparations
------------

Assuming `build-essential`, `openjdk-8-jdk`, `scala` and `sbt` are installed.
You already have this, if you are running a local lila instance.

    ./bin/build-deps.sh  # install scalalib

Install Kyoto cabinet headers and utilities.

    sudo apt-get install libkyotocabinet-dev kyotocabinet-utils

Setup the `$JAVA_HOME` environment variable.

    # on Debian:
    export JAVA_HOME=/usr/lib/jvm/java-8-openjdk-amd64

    # or, on ArchLinux:
    export JAVA_HOME=/usr/lib/jvm/java-8-openjdk

Download and unpack [Kyotocabinet Java package](http://fallabs.com/kyotocabinet/javapkg/).

    curl http://fallabs.com/kyotocabinet/javapkg/kyotocabinet-java-1.24.tar.gz | tar xvz
    cd kyotocabinet-java-1.24
    ./configure
    make
    sudo make install

Run server
----------

    sbt run -Djava.library.path=/usr/local/lib/

Index master games
------------------

    python3 bin/index-pgn.py master <pgn-file>
