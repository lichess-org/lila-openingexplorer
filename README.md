Opening explorer for lichess.org
================================

Preparations
------------

Assuming `build-essential`, `openjdk-8-jdk`, `scala` and `sbt` are installed.
You already have this, if you are running a local lila instance.

    git clone https://github.com/ornicar/scalalib
    cd scalalib
    sbt publish-local

Install Kyoto cabinet headers and utilities.

    sudo apt-get install libkyotocabinet-dev kyotocabinet-utils

Setup `$JAVA_HOME` environment variable.

    export JAVA_HOME=/usr/lib/jvm/java-8-openjdk-amd64
    # or, for ArchLinux:
    export JAVA_HOME=/usr/lib/jvm/java-8-openjdk

Download and unpack [Kyotocabinet Java package](http://fallabs.com/kyotocabinet/javapkg/).

    cd ~
    curl http://fallabs.com/kyotocabinet/javapkg/kyotocabinet-java-1.24.tar.gz | tar xvz
    cd kyotocabinet-java-1.24
    ./configure
    make
    sudo make install

Create empty database
---------------------

    kctreemgr create -bnum 40000000000 bullet.kct

Run server
----------

    sbt -Djava.library.path=/usr/local/lib/ run

Index games
-----------

    python3 index-pgn.py <pgn-file>
