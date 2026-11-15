# Samples

Small inputs for tests and trying out the pipeline and the servers.

## `lichess_db/`

Slices of the monthly dumps of https://database.lichess.org/,
with the same paths and file names (recompressed with `zstd -19`):

* `standard/`: 25,603 games of 2026-10.
* `antichess/`, `atomic/`, `chess960/`, `crazyhouse/`, `horde/`,
  `kingOfTheHill/`, `racingKings/`, `threeCheck/`: 2,236 to 6,025 games
  each (6.7 MiB of PGN) of 2026-10.
* `broadcasts/`: 9,119 games of the 2026-09 broadcast dump

## `masters-db/games/`

Made-up games in the layout of the masters database (`games/yyyy/mm/dd.pgn`,
`xx` for unknown month or day, the same headers, `FEN` and `SetUp` for
Chess960 start positions): random names, events, dates and ratings, legal
random moves, results matching mates.

## `requests/`

Real world requests from the busiest hour on 2026-11-06 (15:20 to 16:20 UTC,
535,915 requests) of the production explorer, for `stress_explorer`:
tab separated
`$msec`, `$status`, `$request_time`, `$upstream_response_time`,
`$request_uri`.

Only `/lichess`, `/lichess/history`, `/masters`, `/master`
and `/masters/pgn/{id}` requests, and only the query parameters the
explorers recognize. No `/player` requests.

## `openings/`

A few opening names in the format of
https://github.com/lichess-org/chess-openings (`a.tsv` to `e.tsv`), for
`--openings-dir`.

## `golden/`

`requests.txt`: real requests of the sample log (and a few more), picked
to cover the lichess and masters explorers: positions with many games, one
game (cold) or none, top and recent games, history, every variant, PGNs, a
bad request.

`responses.txt`: their responses (body, then status and content type)
from databases built from the samples above, which
`samples/test-pipeline.sh` (and CI) compares byte for byte, and checks
against the API description (`doc/specs/`).

After an intended change of responses, record them anew with
`UPDATE_GOLDEN=1 samples/test-pipeline.sh`.
