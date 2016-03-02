Binary packing
==============

Game ref (8 bytes, single game)
-------------------------------

bits | type | description
---- | ---- | ---
   2 | bits | winner: `00` draw, `10` white, `01` black
   2 | bits | speed: `01` bullet, `10` blitz, `11` classical
  12 | uint | average rating of the two players
  48 | uint | game id, base62 decoded with base `0-9a-zA-Z`

Mater database
--------------

### Single game: Game ref (identified by size)

### Up to 5 games: Byte `1`, followed by game refs

### More games:

bytes | type      | description
----- | --------- | ---
    1 | byte      | pack format (`2` to `6`)
    k | uint      | number of white wins
    k | uint      | number of draws
    k | uint      | number of black wins
    6 | uint      | sum of average player ratings for each game
8 * n | game refs | top n game refs

Variable sizes:

pack format | k
--- | ---
2 | 1
3 | 2
4 | 3
5 | 4
6 | 6

Lichess database
----------------

### Single game: Game ref (identified by size)

### Up to 53 games: Byte `1`, followed by game refs

### More games:

bytes | type | description
----- | ---- | ---
    1 | byte | pack format (`2` to `6`)

Then for each of the 15 sub entries Bullet 1600, Blitz 1600, Classical 1600,
Bullet 1800, and so on:

bytes | type | description
----- | ---- | ---
    k | uint | number of white wins
    k | uint | number of draws
    k | uint | number of black wins
    m | uint | sum of the average player ratings for each game

Then:

bytes | type      | description
----- | --------- | ---
8 * n | game refs | recent game refs from each group and top game refs for each speed. newer games first

Variable sizes:

pack format | k | m
--- | --- | ---
2 | 1 | 3
3 | 2 | 4
4 | 3 | 6
5 | 4 | 6
6 | 6 | 6
