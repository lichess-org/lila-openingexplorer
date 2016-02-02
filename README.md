Opening explorer for lichess.org
================================

Preparations
------------

Install libkyotocabinet headers.

    sudo apt-get install libkyotocabinet-dev kyotocabinet-utils

Setup `$JAVA_HOME` environment variable.

    export JAVA_HOME=/usr/lib/jvm/java-8-openjdk-amd64

Download and unpack Kyotocabinet Java package from
http://fallabs.com/kyotocabinet/javapkg/.

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
