Opening explorer for lichess.org
================================

Preparations
------------

Assuming `build-essential`, `openjdk-13-jdk` and `sbt` are installed.
You already have this, if you are running a local lila instance.

Install Kyoto cabinet headers and utilities.

    # Debian:
    sudo apt-get install libkyotocabinet-dev kyotocabinet-utils

    # Arch:
    sudo pacman -S kyotocabinet
    sudo patch -d/ -p0 < kcdbext.h.patch

Setup the `$JAVA_HOME` environment variable.

    # Debian:
    export JAVA_HOME=/usr/lib/jvm/java-13-openjdk-amd64

    # Arch:
    export JAVA_HOME=/usr/lib/jvm/java-13-openjdk

Download and unpack [Kyotocabinet Java package](http://fallabs.com/kyotocabinet/javapkg/).

    wget http://fallabs.com/kyotocabinet/javapkg/kyotocabinet-java-1.24.tar.gz
    sha512sum --check kyotocabinet-java-1.24.tar.gz.sha512
    tar -xvzf kyotocabinet-java-1.24.tar.gz
    cd kyotocabinet-java-1.24
    ./configure
    make
    sudo make install

Create configuration file.

    cp conf/application.conf.example conf/application.conf

Run server
----------

    LD_LIBRARY_PATH=/usr/local/lib sbt run

Index from command line
-----------------------

### Lichess games

    cd index-pgn
    cargo run --release --bin index-lichess -- <games.pgn>

### Master games

    cd index-pgn
    cargo run --release --bin index-master -- <games.pgn>

HTTP API
--------

See https://lichess.org/api#tag/Opening-Explorer. CORS enabled for all domains.

License
-------

lila-openingexplorer is licensed under the AGPLv3+. See LICENSE.txt for the
full license text.
