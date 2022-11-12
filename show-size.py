#!/usr/bin/env python3

import requests

def stat(cf, prop):
    res = requests.get(f"http://localhost:9002/monitor/cf/{cf}/rocksdb.{prop}")
    res.raise_for_status()
    return res.json()

def bytes(num):
    for unit in ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB"]:
        if abs(num) < 1024:
            return "%3.1f %s" % (num, unit)
        num /= 1024

def num(num):
    for unit in ["", "K", "M", "G", "T", "P", "E", "Z", "Y"]:
        if abs(num) < 1000:
            return "%3.1f%s" % (num, unit)
        num /= 1000

num_lichess_games = stat("lichess_game", "estimate-num-keys")
#size_lichess_games = stat("lichess_game", "estimate-live-data-size")
size_lichess_games = stat("lichess_game", "live-sst-files-size")

num_lichess = stat("lichess", "estimate-num-keys")
#size_lichess = stat("lichess", "estimate-live-data-size")
size_lichess = stat("lichess", "live-sst-files-size")

size = size_lichess + size_lichess_games

target = 4_000_000_000

print(f"Games: {num(num_lichess_games)}")
print(f"Size: {bytes(size_lichess_games)}")
print(f"Per game: {bytes(size_lichess_games / num_lichess_games)}")
print("---")
print(f"Positions: {num(num_lichess)}")
print(f"Size: {bytes(size_lichess)}")
print(f"Per game: {bytes(size_lichess / num_lichess_games)}")
print("---")
print(f"Total size: {bytes(size)}")
print(f"Total size per game: {bytes(size / num_lichess_games)}")
print(f"Total size per position: {bytes(size / num_lichess)}")
print("---")
print(f"Projected positions at {num(target)} games: {num(num_lichess / num_lichess_games * target)}")
print(f"Projected size at {num(target)} games: {bytes(size / num_lichess_games * target)}")
