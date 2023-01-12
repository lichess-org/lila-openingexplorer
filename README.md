lila-openingexplorer
====================

[![Test](https://github.com/lichess-org/lila-openingexplorer/actions/workflows/test.yml/badge.svg)](https://github.com/lichess-org/lila-openingexplorer/actions/workflows/test.yml)

[Opening explorer](https://lichess.org/blog/Vs0xMTAAAD4We4Ey/opening-explorer)
for lichess.org, capable of handling trillions of positions, featuring:

* A database of master games
* [Rated games from Lichess itself](https://database.lichess.org/)
* An on-demand database of [openings by player](https://lichess.org/blog/YXMPxxMAACEAy3g4/announcing-the-personal-opening-explorer)

Usage
-----

### Run server

1. Install recent stable Rust ([rustup](https://rustup.rs/) recommended).

2. `git submodule update --init`

3. `ulimit -n 131072 && EXPLORER_LOG=lila_openingexplorer=debug cargo run --release`

:warning: In a production environment, administrative endpoints must be
protected using a reverse proxy.
It's best to whitelist only `/masters`, `/lichess`, and `/player`.

### Index games

1. Download database dumps from https://database.lichess.org/.

2. Index (optionally works directly with compressed files):

   ```
   cd index-pgn
   cargo run --release -- *.pgn.zst
   ```

HTTP API
--------

See https://lichess.org/api#tag/Opening-Explorer.

### `/masters`

### `/lichess`

### `/player`

Example:

```
curl https://explorer.lichess.ovh/player?player=foo&color=white&play=e2e4
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
            "averageOpponentRating": 1500, // or null
            "game": { // only game for this move.
                      // would not actually be sent, because there are multiple
                      // games in this case, but for example:
                "id": "uPdCG6Ts",
                "winner": "black",
                "speed": "correspondence",
                "mode": "casual",
                "white": {
                    "name": "foo",
                    "rating": 1500
                },
                "black": {
                    "name": null,
                    "rating": null
                },
                "year": 2015,
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
            "mode": "casual",
            "white": {
                "name": "foo",
                "rating": 1500
            },
            "black": {
                "name": null,
                "rating": null
            },
            "year": 2015,
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

License
-------

Licensed under the GNU Affero General Public License v3. See the `LICENSE` file
for details.
