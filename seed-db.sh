#!/bin/sh

##############

echo "Importing sample of Lichess games into db..."

curl \
    --range 0-50000000 \
    --remote-name \
    https://database.lichess.org/standard/lichess_db_standard_rated_2026-04.pgn.zst

cargo run --release --manifest-path import-pgn/Cargo.toml -- *.pgn.zst

##############

echo "Importing sample of masters games into db..."

curl \
    --remote-name \
    https://theweekinchess.com/zips/twic1644g.zip

unzip twic1644g.zip

python3 import-master.py twic1644.pgn
