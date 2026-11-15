#!/bin/bash
# Indexing pipeline and servers on the sample data (also run by CI). Needs
# curl, jq and npx (or REDOCLY set to a redocly command).
set -euo pipefail
B=${BIN:-target/release}
W=${WORK:-target/pipeline-test}
rm -rf "$W"; mkdir -p "$W/lichess/sst" "$W/masters/sst"

# lichess
pgns=$(ls samples/lichess_db/*/lichess_db_*_rated_*.pgn.zst | awk -F_ '{print $NF, $0}' | sort -s -k1,1 | cut -d' ' -f2)
$B/pgn_to_parquet --threads 4 --memory-budget 1g --out $W/lichess.parquet $pgns
$B/parquet_to_positions --threads 4 --no-direct-io --scratch $W/scratch --scratch-budget 4g --memory-budget 6g --out $W/lichess/sst $W/lichess.parquet
$B/parquet_to_games --threads 4 --no-direct-io --out $W/lichess/sst/games $W/lichess.parquet
$B/explorer ingest --db $W/lichess/db --sst $W/lichess/sst

# masters
echo deterministic,legacy > $W/legacy_ids.csv
$B/masters_to_parquet samples/masters-db/games --out $W/masters.parquet --legacy-ids $W/legacy_ids.csv
$B/parquet_to_positions --threads 4 --no-direct-io --scratch $W/scratch --scratch-budget 4g --memory-budget 6g --out $W/masters/sst $W/masters.parquet
$B/parquet_to_games --threads 4 --no-direct-io --out $W/masters/sst/games --game-ids $W/masters/sst/game_ids $W/masters.parquet
$B/explorer ingest --db $W/masters/db --sst $W/masters/sst

# player: games of the most active players of the standard sample, from a
# mock of lila's game export
zstdcat samples/lichess_db/standard/*.pgn.zst | sed -n 's/^\[White "\(.*\)"\]$/\1/p' |
    sort | uniq -c | sort -k1,1nr -k2,2 | awk 'NR <= 5 {print $2}' > $W/players.txt
$B/mock_lila prepare --parquet $W/lichess.parquet --players $W/players.txt --out $W/mock_lila --threads 4

# serve and replay the sample requests
$B/explorer lichess --db $W/lichess/db --bind 127.0.0.1:9002 --cache 256m --openings-dir samples/openings --no-blacklist > $W/lichess.log 2>&1 &
lichess=$!
$B/explorer masters --db $W/masters/db --bind 127.0.0.1:9003 --cache 64m --openings-dir samples/openings > $W/masters.log 2>&1 &
masters=$!
$B/mock_lila serve --fixture $W/mock_lila --bind 127.0.0.1:9666 > $W/mock_lila.log 2>&1 &
mock_lila=$!
$B/explorer player --db $W/player/db --bind 127.0.0.1:9004 --cache 64m --lila http://127.0.0.1:9666 --openings-dir samples/openings > $W/player.log 2>&1 &
player=$!
trap 'kill $lichess $masters $mock_lila $player 2>/dev/null || true' EXIT
for port in 9002 9003 9004; do
    for _ in $(seq 100); do
        curl -s -o /dev/null "http://127.0.0.1:$port/" && break
        sleep 0.1
    done
done

# Responses are checked against the API description (doc/specs) by linting
# them as examples of a generated description, which refers to the
# documented response schemas. Undocumented paths, statuses, content types
# and properties are errors.
A=$W/api
mkdir -p $A/examples $A/paths
cp -r doc/specs $A/specs
examples=0
api_example() { # uri, "status content-type", body file
    local path=${1%%\?*} status=${2%% *} media=${2#* } operation
    media=${media%%;*}
    case $path in
        /masters/pgn/*) operation=masters-pgn-gameId ;;
        *) operation=${path#/} ;;
    esac
    examples=$((examples + 1))
    case $media in
        application/json) jq '{value: .}' "$3" ;;
        application/x-ndjson) grep -v '^$' "$3" | tail -n 1 | jq '{value: .}' ;;
        *) jq -Rs '{value: .}' "$3" ;;
    esac > $A/examples/$examples.json
    jq -n --arg n $examples --arg uri "$1" --arg status "$status" --arg media "$media" \
        --arg schema "./specs/tags/openingexplorer/$operation.yaml#/get/responses/$status/content/${media//\//~1}/schema" \
        '{"/\($n)": {get: {operationId: "response\($n)", summary: $uri, responses: {($status): {
            description: $uri,
            content: {($media): {schema: {"$ref": $schema}, examples: {response: {"$ref": "./examples/\($n).json"}}}}
        }}}}}' > $A/paths/$examples.json
}

# Responses of selected requests must equal the recorded ones. To record
# them anew (after an intended change): UPDATE_GOLDEN=1 samples/test-pipeline.sh
for uri in $(grep -v '^#' samples/golden/requests.txt); do
    case $uri in
        /lichess*) port=9002 ;;
        *) port=9003 ;;
    esac
    meta=$(curl -s -o $W/body -w '%{http_code} %{content_type}' "http://127.0.0.1:$port$uri")
    printf 'GET %s\n' "$uri"
    cat $W/body
    printf '\n%s\n\n' "$meta"
    api_example "$uri" "$meta" $W/body
done > $W/responses.txt
if [[ -n ${UPDATE_GOLDEN:-} ]]; then
    cp $W/responses.txt samples/golden/responses.txt
elif ! diff -u samples/golden/responses.txt $W/responses.txt; then
    echo "responses differ from samples/golden/responses.txt"; exit 1
fi

# Fails on invalid responses, server errors and failed requests.
$B/stress_explorer samples/requests/*.tsv.zst \
    --lichess-url http://127.0.0.1:9002 --masters-url http://127.0.0.1:9003 \
    --concurrency 8 --limit 20000 --timeout 30

# The same for /player, in the format of the request logs. Responses
# stream until the player is indexed.
queries=(
    color=white
    'color=black&play=e2e4&recentGames=1'
    'color=white&variant=atomic&modes=rated&speeds=blitz,bullet&since=2026-10'
    'modes=rated' # bad request without color
)
for name in $(cat $W/players.txt) unknown-player; do
    for query in "${queries[@]}"; do
        printf '0\t200\t0\t0\t/player?player=%s&%s\n' "$name" "$query"
    done
done > $W/player-requests.tsv
$B/stress_explorer $W/player-requests.tsv \
    --player-url http://127.0.0.1:9004 --concurrency 8 --timeout 60
for query in "${queries[@]}"; do
    uri="/player?player=$(head -n 1 $W/players.txt)&$query"
    meta=$(curl -s -o $W/body -w '%{http_code} %{content_type}' "http://127.0.0.1:9004$uri")
    api_example "$uri" "$meta" $W/body
done

jq -s '{openapi: "3.1.0", info: {title: "Sample responses", version: "1"},
    servers: [{url: "http://127.0.0.1"}], paths: add}' $A/paths/*.json > $A/responses.json
cat > $A/redocly.yaml <<EOF
extends:
  - minimal
rules:
  no-invalid-media-type-examples:
    severity: error
    allowAdditionalProperties: false
  security-defined: off
  operation-2xx-response: off
EOF
${REDOCLY:-npx --yes @redocly/cli@2.71.2} lint --config $A/redocly.yaml $A/responses.json
if grep -iq panic $W/lichess.log $W/masters.log $W/player.log; then
    echo "server panicked"; exit 1
fi
echo "pipeline ok"
