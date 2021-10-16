lila-openingexplorer3
=====================

Personal opening explorer
[under development](https://github.com/niklasf/lila-openingexplorer3/projects/1).

Usage
-----

```sh
EXPLORER_LOG=lila_openingexplorer3=debug cargo run --release -- --lila https://lichess:***@lichess.dev --bearer lip_***
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
modes | string | *all* | Comma separated list of game modes (`rated`, `casual`) to filter for
speeds | string | *all* | Comma separated list of speeds (`ultraBullet`, `bullet`, `blitz`, `rapid`, `classical`, `correspondence`) to filter for
since | string | `0000/01` | Year/Month. Filter for games played in this month or later
until | string | `3000/12` | Year/Month. Filter for games played in this month or earlier
update | bool | `false` | Index new games from lila

Response: Streamed `application/x-ndjson` with rows as follows.

If `update` has not been requested the stream immediately terminates after
the first row. Otherwise updated rows will be streamed until indexing has been
completed. Updates are throttled. The same row may be repeated to avoid
timeouts.

```js
{
    "white": 10, // total number of white wins from this position
    "draws": 1,
    "black": 22,
    "moves": [
        {
            "uci": "e7e5",
            "san": "e5",
            "white": 6, // total number of white wins with this move.
                        // more may transpose to resulting position.
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
                "month": "2015/09"
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
            "month": "2015/09"
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

Indexing requests are ignored if they are submitted within 60 seconds of the
last indexing request for the same player. Ongoing games are revisited only
once every 24 hours.

Indexing requests are queued. If the queue is full, backpressure is applied to
the indexing request. If the indexing request times out due to backpressure,
it is discarded.

At each point in time, there will never be more than one game stream requested
for each same player, and no more than `--indexers` in total.

Column families
---------------

### `game`

* Key
  * Game ID (6 bytes)
* Value
  * Game information

### `personal`

* Key
  * Hash (12 bytes)
    * Player
    * Color
    * Zobrist hash of position and variant
  * Tree (1 byte)
    * Date
* Value
  * Move
    * Speed
      * Mode
        * Stats
        * Game IDs with sequence number

### `player`

* Key
  * User ID
* Value
  * `createdAt` of last seen game
  * `createdAt` of oldest seen ongoing game
  * Time of last index run

License
-------

Licensed under the GNU Affero General Public License v3. See the `LICENSE` file
for details.
