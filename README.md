lila-openingexplorer3
=====================

Personal opening explorer
[under development](https://github.com/niklasf/lila-openingexplorer3/projects/1).

Usage
-----

```sh
EXPLORER_LOG=lila_openingexplorer3=debug cargo run --release --lila https://lichess:***@lichess.dev
```

HTTP API
--------

Example:

```
curl http://localhost:9000/personal?player=foo&color=white&play=e2e4&update=true
```

Query parameters:

name | type | default | description
--- | --- | --- | ---
variant | string | `chess` | `antichess`, `atomic`, `chess` (or `standard`, `chess960`, `fromPosition`), `crazyhouse`, `horde`, `kingOfTheHill`, `racingKings`, `threeCheck`
fen | string | *starting position of variant* | FEN of the root position
play | string | *empty* | Comma separated moves in UCI notation. Play additional moves starting from *fen*. Required to find an opening name, if *fen* is not an exact match for a named position.
player | string | *required* | Username to filter for
color | string | *required* | Filter for games where *player* is `white` or `black`
modes | string | `rated,casual` | Filter for these game modes
speeds | string | `ultraBullet,bullet,blitz,rapid,classical,correspondence` | Filter for these speeds
since | integer | `2000` | Year. Filter for games played in this year or later
update | bool | `false` | Stream and index new games from lila. The response will be delayed up to 9 seconds, or until all games have been indexed, whichever comes first.

Likely cause of errors:

* 404: Player not found
* 503: Too busy or otherwise unable to index games

Response:

```js
{
    "white": 10, // total number of white wins from this position
    "draws": 1,
    "black": 22,
    "moves": [
        {
            "uci": "e7e5",
            "san": "e5",
            "white": 6,
            "draws": 1,
            "black": 9,
            "game": { // latest game for this move.
                      // perhaps useful to show when it is the only game
                      // for the move
                "id": "uPdCG6Ts",
                "winner": "black",
                "speed": "correspondence",
                "rated": false,
                "white": {
                    "name": "foo",
                    "rating": 1500
                },
                "black": {
                    "name": null,
                    "rating": null
                },
                "year": 2015
            }
        },
        // ...
    ],
    "recentGames": [ // currently up to 15 recent games.
                     // limit is up to discussion.
        {
            "uci": "e7e5",
            "id": "uPdCG6Ts",
            "winner": "black",
            "speed": "correspondence",
            "rated": false,
            "white": {
                "name": "foo",
                "rating": 1500
            },
            "black": {
                "name": null,
                "rating": null
            },
            "year": 2015
        },
        // ...
    ],
    "opening": {
        "eco": "B00",
        "name": "King's Pawn"
    }
}
```

Indexing process
----------------

Currently all indexing requests are queued and performed sequentially.
So players with many games could occupy the queue for a long time. Whether this
will be fine in practice depends on the rate at which the indexer will be
able to stream games from lila.

Column families
---------------

`game`
------

* Key
  * Game ID
* Value
  * Game information

`personal`
----------

* Key
  * Hash
    * Player
    * Color
    * Zobrist hash of position and variant
  * Tree
    * Date
* Value
  * Move
    * Speed
      * Mode
        * Stats
        * Games with sequence number
