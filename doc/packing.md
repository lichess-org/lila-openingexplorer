Pack formats
============

Game refs
---------

2 bit     | 14 bit  | 48 bit
--------- | ------- | -----------------------
00 draw   | rating  | base62 decoded game id
10 white  |         |
01 black  |         |

Nodes
-----

### Single game

A single game ref, detected by size 64 bit.

### Pack format 1 (for up to 5 games)

Byte 1, followed by 5 game refs.

### Pack format 2 (for up to 65536 games)

Byte 2

For each of the 11 rating groups: 16-bit integers with white wins, draws
and black wins.

Followed by a number of game refs.

### Pack format 3

Byte 3

For each of the 11 rating groups: 48-bit integers with white wins, draws
and black wins.

Followed by a number of game refs.
