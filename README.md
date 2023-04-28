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

2. ```
   git submodule update --init
   ```

3. Set some environment variables used at build time: `set -a && source .env && set +a`

4. Build the server and view available options:

   ```
   cargo run --release -- --help
   ```

   Strongly consider adjusting `--db-compaction-readahead`, `--db-cache`, and
   `--db-rate-limit` depending on your setup.

5. Run the server with the chosen options:

   ```
   ulimit -n 131072 && EXPLORER_LOG=lila_openingexplorer=info cargo run --release -- --db-compaction-readahead
   ```

:warning: In a production environment, administrative endpoints must be
protected using a reverse proxy.
It's best to whitelist only `/masters`, `/lichess`, and `/player`.

### Import games

1. Download database dumps from https://database.lichess.org/.

2. Import (optionally works directly with compressed files):

   ```
   cd import-pgn
   cargo run --release -- *.pgn.zst
   ```

   The database size will be well below 3x the compressed PGN size.

   If you can fit this on SSDs, read and compaction performance, especially
   tail latencies, will benefit significantly. All else equal, RAIDs with
   multiple small disks are preferable to RAIDs with few larger disks.
   Block and page cache will take advantage of large amounts of available RAM.

   As of February 2023, Lichess handles around 12k requests/minute, using:

   * 4 spinning disks in RAID10
   * 128 GiB RAM, of which 40 GiB are used for the block cache
   * Compressed PGN import rate of around 100 KiB/s on the live system,
     paused at peak hours.

   Importing is currently very inefficient, but good enough to index faster
   than games are played. Initially, the database was prepared
   offline, with speed dropping as the database grew, averaging 1 MiB/s
   compressed indexing speed (so effectively 7 MiB/s uncompressed PGN data).

Monitoring
----------

### `/monitor`

Example:

```
curl http://localhost:9002/monitor
```

Response in InfluxDB line protocol:

```
opening_explorer block_index_miss=2271815u,block_index_hit=44204637u,block_filter_miss=2272244u,block_filter_hit=81741291u,block_data_miss=31540587u,block_data_hit=33327789u,indexing=5u,lichess_cache=31038u,lichess_miss=2993390u,lichess_history_cache=2112u,lichess_history_miss=19558u,masters_cache=38276u,masters_miss=3430066u,masters=158629555u,masters_game=2519908u,lichess=121970833029u,lichess_game=4331746117u,player=18693470276u,player_status=182129u
```

### `/monitor/db/<prop>`

### `/monitor/cf/<cf>/<prop>`

Example:

```
curl http://localhost:9002/monitor/cf/lichess/rocksdb.stats
```

```
** Compaction Stats [lichess] **
Level    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  L0      1/0   34.75 MB   0.2     64.3     0.0     64.3     157.5     93.2       0.0   1.6      1.4      3.5  46068.07           3002.68      2760   16.691   1763M    92M       0.0       0.0
  L1      4/0   205.48 MB   0.8    172.6    93.2     79.4     165.5     86.1       0.0   1.8      3.7      3.5  48254.63           3779.43       207  233.114   4758M   164M       0.0       0.0
  L2     52/0    2.49 GB   1.0    321.9    81.7    240.3     317.2     77.0       4.4   3.9      3.5      3.4  94232.38           6840.29      1345   70.061   8967M   121M       0.0       0.0
  L3    591/0   24.95 GB   1.0    338.4    78.2    260.1     333.0     72.9       3.1   4.3      3.5      3.5  97870.15           7213.25      1532   63.884   9448M   146M       0.0       0.0
  L4   3367/0   90.71 GB   0.4     81.3    48.6     32.7      79.9     47.2      27.5   1.6      3.7      3.6  22451.14           1770.16       904   24.835   2281M    35M       0.0       0.0
  L6  47776/0    2.78 TB   0.0      0.0     0.0      0.0       0.0      0.0       0.0   0.0      0.0      0.0      0.00              0.00         0    0.000       0      0       0.0       0.0
 Sum  51791/0    2.90 TB   0.0    978.5   301.6    676.9    1053.2    376.3      35.0  10.9      3.2      3.5 308876.38          22605.81      6748   45.773     27G   560M       0.0       0.0
 Int      0/0    0.00 KB   0.0      0.0     0.0      0.0       0.0      0.0       0.0   0.0      0.0      0.0      0.00              0.00         0    0.000       0      0       0.0       0.0

** Compaction Stats [lichess] **
Priority    Files   Size     Score Read(GB)  Rn(GB) Rnp1(GB) Write(GB) Wnew(GB) Moved(GB) W-Amp Rd(MB/s) Wr(MB/s) Comp(sec) CompMergeCPU(sec) Comp(cnt) Avg(sec) KeyIn KeyDrop Rblob(GB) Wblob(GB)
---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
 Low      0/0    0.00 KB   0.0    978.5   301.6    676.9     956.6    279.7       0.0   0.0      3.5      3.4 288675.78          20752.87      4203   68.683     27G   560M       0.0       0.0
High      0/0    0.00 KB   0.0      0.0     0.0      0.0      96.6     96.6       0.0   0.0      0.0      4.9  20200.60           1852.94      2545    7.937       0      0       0.0       0.0

Blob file count: 0, total size: 0.0 GB, garbage size: 0.0 GB, space amp: 0.0

Uptime(secs): 610745.6 total, 3.5 interval
Flush(GB): cumulative 96.595, interval 0.000
AddFile(GB): cumulative 0.000, interval 0.000
AddFile(Total Files): cumulative 0, interval 0
AddFile(L0 Files): cumulative 0, interval 0
AddFile(Keys): cumulative 0, interval 0
Cumulative compaction: 1053.16 GB write, 1.77 MB/s write, 978.51 GB read, 1.64 MB/s read, 308876.4 seconds
Interval compaction: 0.00 GB write, 0.00 MB/s write, 0.00 GB read, 0.00 MB/s read, 0.0 seconds
Stalls(count): 1028 level0_slowdown, 1018 level0_slowdown_with_compaction, 0 level0_numfiles, 0 level0_numfiles_with_compaction, 0 stop for pending_compaction_bytes, 1809 slowdown for pending_compaction_bytes, 0 memtable_compaction, 0 memtable_slowdown, interval 0 total count
Block cache LRUCache@0x7feb60a3f150#4001884 capacity: 40.00 GB usage: 39.97 GB table_size: 524288 occupancy: 6030149 collections: 1018 last_copies: 5 last_secs: 0.039008 secs_since: 408
Block cache entry stats(count,size,portion): DataBlock(165238,10.35 GB,25.885%) FilterBlock(22233,29.11 GB,72.7832%) IndexBlock(22348,499.64 MB,1.21981%) Misc(1,0.00 KB,0%)

** File Read Latency Histogram By Level [lichess] **

** DB Stats **
Uptime(secs): 610745.6 total, 3.5 interval
Cumulative writes: 0 writes, 0 keys, 0 commit groups, 0.0 writes per commit group, ingest: 0.00 GB, 0.00 MB/s
Cumulative WAL: 0 writes, 0 syncs, 0.00 writes per sync, written: 0.00 GB, 0.00 MB/s
Cumulative stall: 00:00:0.000 H:M:S, 0.0 percent
Interval writes: 0 writes, 0 keys, 0 commit groups, 0.0 writes per commit group, ingest: 0.00 MB, 0.00 MB/s
Interval WAL: 0 writes, 0 syncs, 0.00 writes per sync, written: 0.00 GB, 0.00 MB/s
Interval stall: 00:00:0.000 H:M:S, 0.0 percent
```

Public HTTP API
---------------

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
    },
    "queuePosition": 3 // waiting for other players to be indexed first
}
```

License
-------

Licensed under the GNU Affero General Public License v3. See the `LICENSE` file
for details.
