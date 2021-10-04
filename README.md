lila-openingexplorer3
=====================

Goal: Implement personal opening explorer. Likely to become a rewrite of the
existing opening explorer.

Flow
----

Request data for a position (with various filters).
Pull new data from lila on demand.

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
