Opening explorer for lichess.org
================================

[![Build Status](https://travis-ci.org/niklasf/lila-openingexplorer.svg?branch=master)](https://travis-ci.org/niklasf/lila-openingexplorer)

Preparations
------------

Assuming `build-essential`, `openjdk-8-jdk`, `scala` and `sbt` are installed.
You already have this, if you are running a local lila instance.

    cp conf/application.conf.example conf/application.conf # your config
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

    python3 bin/index-pgn.py master <games.pgn>

HTTP API
--------

### `GET /master` query opening database with master games

```
> curl http://explorer.lichess.org/master?fen=rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR%20w%20KQkq%20-%200%201
```

name | type | default | description
--- | --- | --- | ---
**fen** | string | required | FEN of the position to look up
**moves** | int | 12 | number of most common moves to display

```javascript
{
  "total": 1946396,
  "white": 645089,
  "draws": 838975,
  "black": 462332,
  "moves": [  // ordered by total number of games
    {
      "uci": "e2e4",
      "san": "e4",
      "total": 885598,
      "white": 291056,
      "draws": 376163,
      "black": 218379,
      "averageRating": 2401
    },
    {
      "uci": "d2d4",
      "san": "d4",
      "total": 697653,
      "white": 234075,
      "draws": 304135,
      "black": 159443,
      "averageRating": 2414
    },
    // ...
  ],
  "averageRating": 2407,
  "topGames": [  // ordered by average rating, higher first
    {
      "id": "Z67CCrfr",
      "rating": 2849,
      "winner": "white"
    },
    {
      "id": "14datddM",
      "rating": 2848,
      "winner": "draw"
    },
    // ...
  ],
  "recentGames": []  // roughly ordered by date, newer first
}
```

### `GET /master/pgn/{id}` fetch one master game by ID

```
> curl http://explorer.lichess.org/master/pgn/Z67CCrfr
```

```
[Event "Zurich CC Rapid 2014"]
[Site "Zurich SUI"]
[Date "2014.02.04"]
[Round "2.3"]
[White "Aronian, L."]
[Black "Carlsen, M."]
[Result "1-0"]
[WhiteElo "2826"]
[BlackElo "2872"]

1. Nf3 Nf6 2. c4 e6 3. g3 d5 4. Bg2 Be7 5. d4 O-O 6. Qc2 c5 7. O-O cxd4 8. Nxd4 e5 9. Nf5 d4 10. Nxe7+ Qxe7 11. Bg5 h6 12. Bxf6 Qxf6 13. Nd2 Bf5 14. Qb3 Nd7 15. Qa3 Qb6 16. Rfc1 Rfc8 17. b4 a5 18. c5 Qa6 19. Nc4 Be6 20. Nd6 axb4 21. Qxa6 bxa6 22. Nxc8 Rxc8 23. c6 Nb6 24. Rab1 a5 25. a3 b3 1-0
```

### `GET /lichess` query lichess opening database

```
> http://explorer.lichess.org/lichess?variant=standard&speeds[]=blitz&speeds[]=classical&ratings[]=2200&ratings[]=2500&fen=rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR%20w%20KQkq%20-%200%201
```

name | type | default | description
--- | --- | --- | ---
**fen** | string | required | FEN of the position to look up
**variant** | string | required | one of `standard`, `antichess`, `chess960`, `horde`, `racingKings`, `threeCheck`, `atomic`, `crazyhouse` or `kingOfTheHill`
**speeds[]** | list | none | `bullet`, `blitz` and/or `classical`
**ratings[]** | list | none | rating groups ranging from their value to the next higher group: `1600`, `1800`, `2000`, `2200` and `2500` to unlimited
**moves** | int | 12 | number of most common moves to display
