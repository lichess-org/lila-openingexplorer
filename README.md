lila-openingexplorer3
=====================

Personal opening explorer
[under development](https://github.com/niklasf/lila-openingexplorer3/projects/1)
and maybe future replacement for [lila-openingexplorer](https://github.com/niklasf/lila-openingexplorer).

Usage
-----

```sh
git submodule update --init
EXPLORER_LOG=lila_openingexplorer3=debug cargo run -- --lila https://lichess:***@lichess.dev --bearer lip_***
```

HTTP API
--------

Example:

```
curl https://explorer.lichess.ovh/personal?player=foo&color=white&play=e2e4
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
since | string | `0000-01` | Year-Month. Filter for games played in this month or later
until | string | `3000-12` | Year-Month. Filter for games played in this month or earlier

Response: Streamed [`application/x-ndjson`](http://ndjson.org/)
with rows as follows.

Will start indexing, immediately respond with the current results,
and stream more updates until indexing is complete. The stream is throttled
and deduplicated. Empty lines may be sent to avoid timeouts.

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
            "game": { // only game for this move.
                      // would not actually be sent, because there are multiple
                      // in this case, but for example:
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
                "month": "2015-09"
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
            "month": "2015-09"
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

At each point in time, there will never be more than one game stream requested
for each player, and no more than `--indexers` (default 16) in total.

Indexing requests are added to a bounded queue. If the queue is full,
the indexing request will be ignored.

Once started, indexing will run until completed, even if all requesters
disconnect.

Indexing requests are also ignored if they are submitted within 60 seconds of
the last indexing request for the same player, but there can be multiple
concurrent streams watching one indexing process. Ongoing games are revisited
only once every 24 hours.

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
  * Tree (2 byte)
    * Year and month
* Value (>= 12 bytes)
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
