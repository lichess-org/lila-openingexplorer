# lila-openingexplorer

Opening explorer for lichess.org, indexing billions of
chess games (currently ~10 billion rated games across all variants,
and growing).

## API

See the [Lichess API reference](https://lichess.org/api#tag/opening-explorer).

## Serving

### What to expect?

Production easily handles the current peak usage of ~150 requests per second
split across `/lichess`, `/masters` and `/player` on this hardware:

* AMD Ryzen 5 PRO 3600 (12 threads @ 3.6 GHz)
* 128 GiB RAM
* 4x HGST HUS726T6TAL (5.46 TiB) in RAID 10
* 2x WDC CL SN720 SDAQNTW-512G-2000 (477 GiB) in RAID 1

### Preparation

Build the [Rust](https://rust-lang.org/tools/install/) binaries:

```sh
cargo build --release
B=target/release
```

Use the [Hugging Face CLI](https://huggingface.co/docs/huggingface_hub/guides/cli)
to download prepared SSTs (~2.5 TiB) or see indexing instructions below. Example:

```sh
hf sync hf://buckets/nfiekas/opening-explorer-lichess/2026-11-v2 /data/hdd/lichess/bucket/2026-11-v2 --exclude "*.parquet"
```

### Start servers

Create the database directories. The SST files are hard-linked, so each
database must be on the same file system as its SSTs:

```sh
$B/explorer ingest --if-missing --db /data/hdd/lichess/db/2026-11-v2 --sst /data/hdd/lichess/bucket/2026-11-v2
$B/explorer ingest --if-missing --db /data/hdd/masters/db/2026-11 --sst /data/hdd/masters/sst/2026-11
```

Serve them:

```sh
$B/explorer lichess --db /data/hdd/lichess/db/2026-11-v2 \
    --bind 127.0.0.1:9002 --admin-bind 127.0.0.1:9102 \
    --cors \
    --cache 16g --pin-metadata \
    --parallel-fetch --blocking-threads 16 \
    --no-blacklist

$B/explorer masters --db /data/hdd/masters/db/2026-11 \
    --bind 127.0.0.1:9003 --admin-bind 127.0.0.1:9103 \
    --cors \
    --cache 4g \
    --blocking-threads 12

$B/explorer player --db /data/ssd/player/db \
    --bind 127.0.0.1:9004 --admin-bind 127.0.0.1:9104 \
    --cors \
    --db-path /data/ssd/player/sst:64g --db-path /data/hdd/player/sst:4t \
    --cache 4g \
    --lila-bearer-file /etc/explorer-bearer
```

* `--cache`: block caches total only 24 GiB, so that most RAM is page cache.
  As long as meta data is cached, the page cache (compressed blocks) saves
  about as many disk reads per GiB as the block cache, survives restarts,
  and leaves room for a second lichess instance during the monthly switch.
  No `--direct-reads` to use page cache.
* `--pin-metadata`: Loads and pins all index and filter blocks at startup
  (~1 min on the HDDs, 10.2 GiB as of 2026-11, growing with the number of
  games), so that a lookup needs at most one data block read. +30%
  throughput from a cold start. `--cache` must exceed the metadata with
  some room for data blocks.
* `--parallel-fetch`: Reads the games of a response concurrently. Useful for
  improved I/O scheduling on HDD.
* `--blocking-threads`: It can be useful to oversubscribe as configured here.
* `--response-cache`: The default of 100000 had a 28% hit rate in 2026-11.
  More gains little.
* `--no-blacklist` / `--lila-bearer-file`: The official Lichess instance
  gets to use a blacklist with players marked by moderators and relaxed
  internal rate limits for personal explorer indexing.
* `--admin-bind`: Prometheus metrics at `/metrics` (and
  `DELETE /player/{id}` for the personal explorer). Keep private.
* `--db-path` for the personal explorer: The write ahead log stays in
  `--db`, upper levels go to the NVMe and the last level (about 10x
  larger, see `--level-ratio`) to the HDDs. Each column family places its
  levels against the whole size of a path, so a young database can put up
  to about 3x the first path's size on it.
* `--rate-limit`: Defaults to 16 MiB/s, so player database compactions do not
  disturb the other servers too much.

### Testing

Replay an nginx request log (tab separated `$msec`, `$status`,
`$request_time`, `$upstream_response_time`, `$request_uri`) against the
servers:

```sh
$B/stress_explorer samples/requests/explorer-2026-11-06T1520Z-1h.tsv.zst \
    --lichess-url http://127.0.0.1:9002 \
    --masters-url http://127.0.0.1:9003 \
    --player-url http://127.0.0.1:9004
```

## Indexing

### What to expect?

Indexing run of ~2.4 TiB input PGNs

| Step | Time | Peak RSS |
| --- | --- | --- |
| `pgn_to_parquet` | 2 h 08 min | 28 GiB |
| `parquet_to_positions` (3 passes, `--scratch-budget 2863g`) | 2 h 40 min | 80 GiB |
| `parquet_to_games` | 15 min | 2 GiB |
| `ingest` | 8 s | |

on this development hardware:

* AMD Ryzen 9 9950X (16 cores, 32 threads), `--threads 30`
* 192 GiB RAM, `--memory-budget 100g`
* Input PGNs on HDD readable at ~330 MiB/s sequential
* Output and scratch space on 7.3 TiB NVMe, sustaining ~2.6 GiB/s reads +
  2.7 GiB/s writes concurrently.

### `pgn_to_parquet`

Prepare a parquet file from PGNs for further processing.

**Input:** ~2.4 TiB of monthly game dumps from https://database.lichess.org \
**Output:** ~470 GiB parquet file \
**Typical bottleneck:** sequential reads of the dumps (from HDDs) > CPU

Inputs must be in chronological order by month.

Example:

```sh
pgns=$(ls samples/lichess_db/*/lichess_db_*_rated_*.pgn.zst | awk -F_ '{print $NF, $0}' | sort -s -k1,1 | cut -d' ' -f2)
$B/pgn_to_parquet --out lichess/lichess_db_rated.parquet $pgns
```

### `parquet_to_positions`

Scan out position SSTs from the parquet file.

**Input:** ~470 GiB parquet file from above \
**Output:** ~2 TiB of SST files \
**Additional resources:** ideally ~5.5 TiB fast disk scratch space (for a
single pass, ~3 TiB for 2 passes, ~1.8 TiB for 3), ~100 GiB RAM \
**Typical bottleneck:** CPU, because every pass replays all games (~50 min per
pass on the development hardware), scratch space determining the number of
passes > scratch write throughput (a single pass writes ~5 TiB) > memory

Walk the parquet file to produce SSTs indexed by position. Can walk the
parquet multiple times to make do with insufficient scratch space.

Scratch space on hard disks works, but then the run is bound by sequential
disk throughput: all scratch data is written once and read once, however many
passes there are. Scratch and SST files are written through the page cache,
so that kernel writeback keeps the many concurrently written files
sequential.

Example:

```sh
$B/parquet_to_positions lichess/lichess_db_rated.parquet --out lichess/sst --scratch scratch --scratch-budget 6000g --memory-budget 100g
```

### `parquet_to_games`

Scan out game SSTs

**Input:** ~470 GiB parquet file from above \
**Output:** ~560 GiB of SST files \
**Additional resources:** no scratch space, ~2 GiB RAM \
**Typical bottleneck:** CPU (compression)

```sh
$B/parquet_to_games lichess/lichess_db_rated.parquet --out lichess/sst/games
```

## Todo

* [ ] Broadcast games
* [x] Unify masters/lichess API
* [ ] Upstream RocksDB PerfStatsLevel mismatch
* [x] Document and use title on GamePlayer
* [ ] Record provenance in SST file meta data
* [x] Delta-encoding for intermediate entries
* [ ] Consider introducing player ids
* [ ] Explore more expensive move compression for cold storage

## License

Licensed under the GNU Affero General Public License v3. See the `LICENSE` file
for details.
