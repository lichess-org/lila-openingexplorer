#!/bin/sh -e
ulimit -n 16000
cargo build --release
python fens.py &
cargo flamegraph --bin lila-openingexplorer
