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
curl http://localhost:9000/personal?player=foo&color=white&update=true
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

Likely cause of status codes:

* 404: Player not found
* 503: Too busy or otherwise unable to index games

Data
----

### Explorer

#### Hashed

* Position (incl. variant)
* Player
* White/Black

#### Lexicographic

* Date
* Move

#### Multiple select

* Speed
* Rated/Casual

#### Value

* Win/Draw/Loss
* Rating
* Games: IDs, Opponents with ratings

### Player

* Last game ID
* Last pulled
* Name
* Title
* Erased

### Game

* White player
* White rating
* Black player
* Black rating
